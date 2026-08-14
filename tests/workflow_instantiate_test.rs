mod common;

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use common::TestProject;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;

fn write_project_file(workspace: &Path, relative_path: &str, contents: &str) {
    let path = workspace.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn write_workflow_fixture(project: &TestProject) {
    write_project_file(
        project.path(),
        ".task-golem/plugins/writer.yaml",
        "version: 1\nargv: [\"python3\", \"scripts/writer.py\"]\n",
    );
    write_project_file(
        project.path(),
        ".task-golem/workflows/example.yaml",
        r#"
version: 1
name: example
plugins:
  writer: .task-golem/plugins/writer.yaml
nodes:
  - id: campaign
    kind: container
    title: ${change}
  - id: story
    kind: container
    parent: campaign
    title: Story
  - id: write
    kind: task
    parent: story
    title: Write
    description: Do the work
    plugin: writer
    context: shared:story
    input:
      change: ${change}
    verify: ["just", "check"]
  - id: review
    kind: task
    parent: story
    depends_on: [write]
    title: Review
    plugin: writer
    context: fresh
"#,
    );
}

fn instantiate(project: &TestProject, change: &str) -> std::process::Output {
    instantiate_with_inputs(project, &[&format!("change={change}")])
}

fn instantiate_with_inputs(project: &TestProject, inputs: &[&str]) -> std::process::Output {
    let mut args = vec![
        "--json",
        "workflow",
        "instantiate",
        ".task-golem/workflows/example.yaml",
        "--instance",
        "WRK-123",
    ];
    for input in inputs {
        args.extend(["--input", input]);
    }
    project.run_tg(&args)
}

fn parse_json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON output: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn spawn_barriered_instantiate(project: &TestProject) -> Child {
    let tg = project.tg_cmd();
    Command::new("sh")
        .args([
            "-c",
            "printf 'READY\\n' >&2; IFS= read -r release; exec \"$@\"",
            "workflow-instantiate-test",
        ])
        .arg(tg.get_program())
        .args([
            "--json",
            "workflow",
            "instantiate",
            ".task-golem/workflows/example.yaml",
            "--instance",
            "WRK-123",
            "--input",
            "change=TG-009",
        ])
        .current_dir(project.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn assert_ready(child: &mut Child) {
    let mut ready = [0; 6];
    child
        .stderr
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(&ready, b"READY\n");
}

fn release(child: &mut Child) {
    child.stdin.take().unwrap().write_all(b"release\n").unwrap();
}

fn assert_persisted_graph_rejected(case: &str, mutate: impl FnOnce(&mut Vec<Item>)) {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    assert!(instantiate(&project, "TG-009").status.success());
    let store = Store::new(project.project_dir());
    let mut items = store.load_active().unwrap();
    mutate(&mut items);
    store.save_active(&items).unwrap();
    let tasks_path = store.tasks_path();
    let altered_tasks = fs::read(&tasks_path).unwrap();
    let archive_path = store.archive_path();
    let original_archive = fs::read(&archive_path).unwrap();

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert_eq!(output.status.code(), Some(2), "case: {case}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match its expected graph")
            || stderr.contains("duplicate item ID"),
        "case: {case}; got: {stderr}"
    );
    assert_eq!(fs::read(tasks_path).unwrap(), altered_tasks);
    assert_eq!(fs::read(archive_path).unwrap(), original_archive);
}

#[test]
fn instantiate_creates_the_resolved_graph_and_workflow_projection() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert!(
        output.status.success(),
        "instantiate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = parse_json(&output);
    let digest = result["instance_digest"].as_str().unwrap();
    assert!(digest.starts_with("sha256:"));
    assert_eq!(digest.len(), "sha256:".len() + 64);
    assert_eq!(result["instance"], "WRK-123");
    assert_eq!(result["campaign_id"], result["nodes"]["campaign"]);
    assert_eq!(
        result["nodes"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["campaign", "review", "story", "write"]
    );

    let items = Store::new(project.project_dir()).load_active().unwrap();
    assert_eq!(items.len(), 4);
    let node_id = |node: &str| result["nodes"][node].as_str().unwrap();
    let item = |node: &str| items.iter().find(|item| item.id == node_id(node)).unwrap();

    assert_eq!(item("campaign").title, "TG-009");
    assert_eq!(item("campaign").parent, None);
    assert_eq!(item("story").parent.as_deref(), Some(node_id("campaign")));
    assert_eq!(item("write").parent.as_deref(), Some(node_id("story")));
    assert_eq!(item("review").dependencies, [node_id("write")]);

    for node in ["campaign", "story", "write", "review"] {
        let item = item(node);
        assert_eq!(item.status, Status::Todo);
        assert_eq!(item.priority, 0);
        assert_eq!(item.blocked_reason, None);
        assert_eq!(item.blocked_from_status, None);
        assert_eq!(item.claimed_by, None);
        assert_eq!(item.claimed_at, None);
        assert!(item.tags.is_empty());
        assert_eq!(item.created_at, item.updated_at);
    }
    assert_eq!(
        item("campaign").extensions["x-workflow"],
        serde_json::json!({
            "version": 1,
            "instance": "WRK-123",
            "node": "campaign",
            "kind": "container",
            "instance_digest": digest
        })
    );
    assert_eq!(
        item("story").extensions["x-workflow"],
        serde_json::json!({
            "version": 1,
            "instance": "WRK-123",
            "node": "story",
            "kind": "container",
            "instance_digest": digest
        })
    );
    assert_eq!(
        item("write").extensions["x-workflow"],
        serde_json::json!({
            "version": 1,
            "instance": "WRK-123",
            "node": "write",
            "kind": "task",
            "plugin": {
                "version": 1,
                "argv": ["python3", "scripts/writer.py"]
            },
            "context": {
                "mode": "shared",
                "key": "story"
            },
            "input": {
                "change": "TG-009"
            },
            "verify": ["just", "check"],
            "instance_digest": digest
        })
    );
    assert_eq!(
        item("review").extensions["x-workflow"],
        serde_json::json!({
            "version": 1,
            "instance": "WRK-123",
            "node": "review",
            "kind": "task",
            "plugin": {
                "version": 1,
                "argv": ["python3", "scripts/writer.py"]
            },
            "context": {"mode": "fresh"},
            "input": {},
            "instance_digest": digest
        })
    );
}

#[test]
fn instantiate_human_output_identifies_the_campaign_and_sorted_node_map() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let json_output = instantiate(&project, "TG-009");
    assert!(json_output.status.success());
    let result = parse_json(&json_output);

    // Act
    let output = project.run_tg(&[
        "workflow",
        "instantiate",
        ".task-golem/workflows/example.yaml",
        "--instance",
        "WRK-123",
        "--input",
        "change=TG-009",
    ]);

    // Assert
    assert!(output.status.success());
    let expected = format!(
        "Campaign: {}\nInstance: WRK-123\nDigest: {}\nNodes:\n  campaign: {}\n  review: {}\n  story: {}\n  write: {}\n",
        result["campaign_id"].as_str().unwrap(),
        result["instance_digest"].as_str().unwrap(),
        result["nodes"]["campaign"].as_str().unwrap(),
        result["nodes"]["review"].as_str().unwrap(),
        result["nodes"]["story"].as_str().unwrap(),
        result["nodes"]["write"].as_str().unwrap(),
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn matching_active_instance_is_byte_stable_and_mismatch_does_not_write() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let first = instantiate(&project, "TG-009");
    assert!(first.status.success());
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let original_tasks = fs::read(&tasks_path).unwrap();

    // Verify a matching instance returns the same deterministic result without a write.
    let matching = instantiate(&project, "TG-009");
    assert!(matching.status.success());
    assert_eq!(matching.stdout, first.stdout);
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);

    // Verify changed resolved input rejects the existing key without a write.
    let mismatch = instantiate(&project, "TG-010");
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&mismatch.stderr).contains("different digest"),
        "got: {}",
        String::from_utf8_lossy(&mismatch.stderr)
    );
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);

    // Verify changed resolved plugin material also rejects without a write.
    write_project_file(
        project.path(),
        ".task-golem/plugins/writer.yaml",
        "version: 1\nargv: [\"python3\", \"scripts/reviewer.py\"]\n",
    );
    let plugin_mismatch = instantiate(&project, "TG-009");
    assert_eq!(plugin_mismatch.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&plugin_mismatch.stderr).contains("different digest"));
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);
}

#[test]
fn changed_graph_definition_rejects_the_existing_key_without_a_write() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    assert!(instantiate(&project, "TG-009").status.success());
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let original_tasks = fs::read(&tasks_path).unwrap();
    let template_path = project.path().join(".task-golem/workflows/example.yaml");
    let template = fs::read_to_string(&template_path).unwrap();
    fs::write(
        template_path,
        template.replace("    depends_on: [write]\n", "    depends_on: []\n"),
    )
    .unwrap();

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("different digest"));
    assert_eq!(fs::read(tasks_path).unwrap(), original_tasks);
}

#[test]
fn multi_input_argument_order_does_not_change_the_instance_digest() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_project_file(
        project.path(),
        ".task-golem/plugins/writer.yaml",
        "version: 1\nargv: [\"writer\"]\n",
    );
    write_project_file(
        project.path(),
        ".task-golem/workflows/example.yaml",
        r#"
version: 1
name: example
plugins:
  writer: .task-golem/plugins/writer.yaml
nodes:
  - id: campaign
    kind: container
    title: ${title}
  - id: write
    kind: task
    parent: campaign
    title: Write
    plugin: writer
    context: fresh
    input:
      owner: ${owner}
"#,
    );

    // Act
    let first = instantiate_with_inputs(&project, &["title=Campaign", "owner=HAMY"]);
    let second = instantiate_with_inputs(&project, &["owner=HAMY", "title=Campaign"]);

    // Assert
    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(parse_json(&first), parse_json(&second));
    assert_eq!(
        Store::new(project.project_dir())
            .load_active()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn concurrent_same_key_instantiation_creates_one_graph() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let mut first = spawn_barriered_instantiate(&project);
    let mut second = spawn_barriered_instantiate(&project);
    assert_ready(&mut first);
    assert_ready(&mut second);

    // Act: release both ready processes to contend for the Store lock.
    release(&mut first);
    release(&mut second);
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    // Assert
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(parse_json(&first), parse_json(&second));
    assert_eq!(
        Store::new(project.project_dir())
            .load_active()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn persisted_instance_must_match_the_exact_graph_definition() {
    assert_persisted_graph_rejected("missing node", |items| {
        items.retain(|item| item.extensions["x-workflow"]["node"] != "write");
    });
    assert_persisted_graph_rejected("extra node", |items| {
        let mut extra = items[0].clone();
        extra.id = "tg-extra".to_string();
        extra.extensions.get_mut("x-workflow").unwrap()["node"] = serde_json::json!("extra");
        items.push(extra);
    });
    assert_persisted_graph_rejected("non-injective item IDs", |items| {
        let write_id = items
            .iter()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap()
            .id
            .clone();
        let review = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "review")
            .unwrap();
        review.id = write_id;
    });
    assert_persisted_graph_rejected("node identity", |items| {
        let review = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "review")
            .unwrap();
        review.extensions.get_mut("x-workflow").unwrap()["kind"] = serde_json::json!("container");
    });
    assert_persisted_graph_rejected("title", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.title = "Altered".to_string();
    });
    assert_persisted_graph_rejected("description", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.description = None;
    });
    assert_persisted_graph_rejected("plugin", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.extensions.get_mut("x-workflow").unwrap()["plugin"]["argv"] =
            serde_json::json!(["altered"]);
    });
    assert_persisted_graph_rejected("context", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.extensions.get_mut("x-workflow").unwrap()["context"] =
            serde_json::json!({"mode": "none"});
    });
    assert_persisted_graph_rejected("input", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.extensions.get_mut("x-workflow").unwrap()["input"]["change"] =
            serde_json::json!("altered");
    });
    assert_persisted_graph_rejected("verify", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.extensions.get_mut("x-workflow").unwrap()["verify"] = serde_json::json!(["altered"]);
    });
    assert_persisted_graph_rejected("parent edge", |items| {
        let review = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "review")
            .unwrap();
        review.parent = None;
    });
    assert_persisted_graph_rejected("dependency edge", |items| {
        let review = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "review")
            .unwrap();
        review.dependencies.clear();
    });
    assert_persisted_graph_rejected("Task session_ref", |items| {
        let write = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "write")
            .unwrap();
        write.extensions.get_mut("x-workflow").unwrap()["session_ref"] =
            serde_json::json!("opaque-session");
    });
    assert_persisted_graph_rejected("non-owner container session_ref", |items| {
        let campaign = items
            .iter_mut()
            .find(|item| item.extensions["x-workflow"]["node"] == "campaign")
            .unwrap();
        campaign.extensions.get_mut("x-workflow").unwrap()["session_ref"] =
            serde_json::json!("opaque-session");
    });
}

#[test]
fn persisted_runtime_state_and_future_session_reference_do_not_trigger_a_write() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let first = instantiate(&project, "TG-009");
    assert!(first.status.success());
    let store = Store::new(project.project_dir());
    let mut items = store.load_active().unwrap();
    let write = items
        .iter_mut()
        .find(|item| item.extensions["x-workflow"]["node"] == "write")
        .unwrap();
    write.status = Status::Doing;
    write.claimed_by = Some("workflow:campaign".to_string());
    let story = items
        .iter_mut()
        .find(|item| item.extensions["x-workflow"]["node"] == "story")
        .unwrap();
    story.extensions.get_mut("x-workflow").unwrap()["session_ref"] =
        serde_json::json!("opaque-session");
    store.save_active(&items).unwrap();
    let tasks_path = store.tasks_path();
    let altered_tasks = fs::read(&tasks_path).unwrap();

    // Act
    let matching = instantiate(&project, "TG-009");

    // Assert
    assert!(
        matching.status.success(),
        "{}",
        String::from_utf8_lossy(&matching.stderr)
    );
    assert_eq!(matching.stdout, first.stdout);
    assert_eq!(fs::read(tasks_path).unwrap(), altered_tasks);
}

#[test]
fn every_workflow_record_is_parsed_before_instance_filtering() {
    for (case, metadata) in [
        ("malformed", serde_json::json!("not-an-object")),
        (
            "missing instance",
            serde_json::json!({
                "version": 1,
                "node": "other",
                "kind": "container",
                "instance_digest": "sha256:other"
            }),
        ),
        (
            "non-string instance",
            serde_json::json!({
                "version": 1,
                "instance": 123,
                "node": "other",
                "kind": "container",
                "instance_digest": "sha256:other"
            }),
        ),
    ] {
        // Arrange
        let project = TestProject::new().unwrap();
        write_workflow_fixture(&project);
        let added = project.run_tg(&["--json", "add", "Existing workflow record"]);
        assert!(added.status.success());
        let store = Store::new(project.project_dir());
        let mut items = store.load_active().unwrap();
        items[0]
            .extensions
            .insert("x-workflow".to_string(), metadata);
        store.save_active(&items).unwrap();
        let tasks_path = store.tasks_path();
        let original_tasks = fs::read(&tasks_path).unwrap();
        let archive_path = store.archive_path();
        let original_archive = fs::read(&archive_path).unwrap();

        // Act
        let output = instantiate(&project, "TG-009");

        // Assert
        assert_eq!(output.status.code(), Some(2), "case: {case}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid x-workflow metadata"),
            "case: {case}; got: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(tasks_path).unwrap(), original_tasks);
        assert_eq!(fs::read(archive_path).unwrap(), original_archive);
    }
}

#[test]
fn non_workflow_items_do_not_prevent_instantiation() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let added = project.run_tg(&["add", "Ordinary task"]);
    assert!(added.status.success());

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert!(output.status.success());
    assert_eq!(
        Store::new(project.project_dir())
            .load_active()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn duplicate_item_ids_across_active_and_archive_reject_before_idempotent_return() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    assert!(instantiate(&project, "TG-009").status.success());
    let store = Store::new(project.project_dir());
    let duplicate = store.load_active().unwrap().remove(0);
    store.append_to_archive(&duplicate).unwrap();
    let tasks_path = store.tasks_path();
    let original_tasks = fs::read(&tasks_path).unwrap();
    let archive_path = store.archive_path();
    let original_archive = fs::read(&archive_path).unwrap();

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate item ID"));
    assert_eq!(fs::read(tasks_path).unwrap(), original_tasks);
    assert_eq!(fs::read(archive_path).unwrap(), original_archive);
}

#[test]
fn instantiation_fails_closed_on_a_malformed_archive_record() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let store = Store::new(project.project_dir());
    let mut archive = fs::OpenOptions::new()
        .append(true)
        .open(store.archive_path())
        .unwrap();
    writeln!(archive, "{{malformed").unwrap();
    archive.sync_all().unwrap();
    assert!(store.load_all_archive().unwrap().is_empty());
    let tasks_path = store.tasks_path();
    let original_tasks = fs::read(&tasks_path).unwrap();
    let archive_path = store.archive_path();
    let malformed_archive = fs::read(&archive_path).unwrap();

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Malformed archive item"));
    assert_eq!(fs::read(tasks_path).unwrap(), original_tasks);
    assert_eq!(fs::read(archive_path).unwrap(), malformed_archive);
}

#[test]
fn instantiation_rejects_missing_or_empty_archive_without_a_write() {
    for (case, archive_contents) in [("missing", None), ("empty", Some(""))] {
        // Arrange
        let project = TestProject::new().unwrap();
        write_workflow_fixture(&project);
        let store = Store::new(project.project_dir());
        let archive_path = store.archive_path();
        match archive_contents {
            Some(contents) => fs::write(&archive_path, contents).unwrap(),
            None => fs::remove_file(&archive_path).unwrap(),
        }
        let tasks_path = store.tasks_path();
        let original_tasks = fs::read(&tasks_path).unwrap();

        // Act
        let output = instantiate(&project, "TG-009");

        // Assert
        assert_eq!(output.status.code(), Some(2), "case: {case}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Archive file"),
            "case: {case}; got: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(tasks_path).unwrap(), original_tasks);
        match archive_contents {
            Some(contents) => assert_eq!(fs::read_to_string(archive_path).unwrap(), contents),
            None => assert!(!archive_path.exists()),
        }
    }
}

#[test]
fn instantiation_accepts_a_valid_header_only_archive() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let store = Store::new(project.project_dir());
    let archive_path = store.archive_path();
    let original_archive = fs::read(&archive_path).unwrap();
    assert!(store.load_all_archive_strict().unwrap().is_empty());

    // Act
    let output = instantiate(&project, "TG-009");

    // Assert
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(archive_path).unwrap(), original_archive);
}

#[test]
fn matching_archived_campaign_is_returned_and_still_guards_the_instance_key() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let first = instantiate(&project, "TG-009");
    assert!(first.status.success());
    let first_json = parse_json(&first);
    let campaign_id = first_json["campaign_id"].as_str().unwrap();
    let archived = project.run_tg(&["done", campaign_id]);
    assert!(archived.status.success());
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let archive_path = project.project_dir().join("archive.jsonl");
    let original_tasks = fs::read(&tasks_path).unwrap();
    let original_archive = fs::read(&archive_path).unwrap();

    // Verify the archived Campaign makes an exact repeat idempotent.
    let matching = instantiate(&project, "TG-009");
    assert!(matching.status.success());
    assert_eq!(matching.stdout, first.stdout);
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);
    assert_eq!(fs::read(&archive_path).unwrap(), original_archive);

    // Verify the archived Campaign reserves the instance key for its digest.
    let mismatch = instantiate(&project, "TG-010");
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("different digest"));
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);
    assert_eq!(fs::read(&archive_path).unwrap(), original_archive);
}

#[test]
fn invalid_definition_fails_before_any_store_write() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_workflow_fixture(&project);
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let original_tasks = fs::read(&tasks_path).unwrap();
    write_project_file(
        project.path(),
        ".task-golem/workflows/example.yaml",
        "version: 1\nname: invalid\nplugins: {}\nnodes: []\n",
    );

    // Act
    let output = project.run_tg(&[
        "--json",
        "workflow",
        "instantiate",
        ".task-golem/workflows/example.yaml",
        "--instance",
        "WRK-123",
    ]);

    // Assert
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("must declare at least one plugin"));
    assert_eq!(fs::read(&tasks_path).unwrap(), original_tasks);
}
