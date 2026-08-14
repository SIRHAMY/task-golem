mod common;

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{TimeZone, Utc};
use common::TestProject;
use sha2::{Digest, Sha256};
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::workflow::runner::{WorkflowRunKind, run_campaign_state_shell};

fn write_project_file(workspace: &Path, relative_path: &str, contents: &str) {
    let path = workspace.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn write_fixture(project: &TestProject) {
    write_project_file(
        project.path(),
        ".task-golem/plugins/worker.yaml",
        "version: 1\nargv: [\"worker\"]\n",
    );
    write_project_file(
        project.path(),
        ".task-golem/workflows/runner.yaml",
        r#"
version: 1
name: runner
plugins:
  worker: .task-golem/plugins/worker.yaml
nodes:
  - id: campaign
    kind: container
    title: Campaign
  - id: story
    kind: container
    parent: campaign
    title: Story
  - id: first
    kind: task
    parent: story
    title: First
    plugin: worker
    context: fresh
  - id: second
    kind: task
    parent: story
    depends_on: [first]
    title: Second
    plugin: worker
    context: fresh
  - id: third
    kind: task
    parent: story
    title: Third
    plugin: worker
    context: fresh
"#,
    );
}

fn instantiate(project: &TestProject, instance: &str) -> serde_json::Value {
    let output = project.run_tg(&[
        "--json",
        "workflow",
        "instantiate",
        ".task-golem/workflows/runner.yaml",
        "--instance",
        instance,
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn item_id(instance: &serde_json::Value, node: &str) -> String {
    instance["nodes"][node].as_str().unwrap().to_string()
}

fn run_shell(
    project: &TestProject,
    campaign_id: &str,
) -> task_golem::workflow::runner::WorkflowRunOutcome {
    run_campaign_state_shell(&Store::new(project.project_dir()), campaign_id).unwrap()
}

fn campaign_lock_path(results_dir: &Path, campaign_id: &str) -> PathBuf {
    let digest = Sha256::digest(campaign_id.as_bytes());
    results_dir.join(format!("campaign-{digest:x}.lock"))
}

fn workflow_bytes(project: &TestProject) -> Vec<Vec<u8>> {
    let project_dir = project.project_dir();
    [
        "tasks.jsonl",
        "archive.jsonl",
        "events.jsonl",
        "events.archive.jsonl",
    ]
    .into_iter()
    .map(|name| fs::read(project_dir.join(name)).unwrap_or_default())
    .collect()
}

fn archive_item(store: &Store, item_id: &str) {
    let mut active = store.load_active().unwrap();
    let index = active.iter().position(|item| item.id == item_id).unwrap();
    let mut item = active.remove(index);
    item.status = Status::Done;
    item.claimed_by = None;
    item.claimed_at = None;
    store.append_to_archive(&item).unwrap();
    store.save_active(&active).unwrap();
}

#[test]
fn claims_the_scoped_ready_task_using_ready_order_and_id_tiebreaker() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let third_id = item_id(&instance, "third");
    assert!(project.run_tg(&["add", "unrelated"]).status.success());
    let store = Store::new(project.project_dir());
    let mut active = store.load_active().unwrap();
    let timestamp = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    for item in &mut active {
        if item.id == first_id || item.id == second_id || item.id == third_id {
            item.priority = 3;
            item.created_at = timestamp;
        }
        if item.title == "unrelated" {
            item.priority = 99;
        }
    }
    store.save_active(&active).unwrap();

    // Act
    let outcome = run_shell(&project, &campaign_id);

    // Assert
    let expected = [first_id.as_str(), third_id.as_str()]
        .into_iter()
        .min()
        .unwrap();
    assert_eq!(outcome.outcome, WorkflowRunKind::Claimed);
    assert_eq!(outcome.task_id.as_deref(), Some(expected));
    let active = store.load_active().unwrap();
    let selected = active.iter().find(|item| item.id == expected).unwrap();
    assert_eq!(selected.status, Status::Doing);
    assert_eq!(
        selected.claimed_by.as_deref(),
        Some(format!("workflow:{campaign_id}").as_str())
    );
    assert_eq!(
        active
            .iter()
            .find(|item| item.id == second_id)
            .unwrap()
            .status,
        Status::Todo
    );
    assert_eq!(
        active
            .iter()
            .find(|item| item.title == "unrelated")
            .unwrap()
            .status,
        Status::Todo
    );
}

#[test]
fn resumes_one_matching_doing_task() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let store = Store::new(project.project_dir());
    let mut active = store.load_active().unwrap();
    let claim = format!("workflow:{campaign_id}");
    for item in &mut active {
        if item.id == first_id {
            item.apply_do(Some(claim.clone())).consume_for_test();
        }
    }
    store.save_active(&active).unwrap();

    // Act
    let resumed = run_shell(&project, &campaign_id);

    // Assert
    assert_eq!(resumed.outcome, WorkflowRunKind::Resume);
    assert_eq!(resumed.task_id.as_deref(), Some(first_id.as_str()));
}

#[test]
fn rejects_ambiguous_doing_tasks() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let store = Store::new(project.project_dir());
    let mut active = store.load_active().unwrap();
    let claim = format!("workflow:{campaign_id}");
    for item in &mut active {
        if item.id == first_id {
            item.apply_do(Some(claim.clone())).consume_for_test();
        }
        if item.id == second_id {
            item.apply_do(None).consume_for_test();
        }
    }
    store.save_active(&active).unwrap();

    // Act
    let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

    // Assert
    assert!(error.to_string().contains("ambiguous doing Tasks"));
}

#[test]
fn rejects_out_of_scope_children() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let store = Store::new(project.project_dir());
    let mut active = store.load_active().unwrap();
    let mut outsider = active
        .iter()
        .find(|item| item.id == first_id)
        .unwrap()
        .clone();
    outsider.id = "tg-outsider".to_string();
    outsider.parent = Some(campaign_id.clone());
    outsider.extensions.clear();
    active.push(outsider);
    store.save_active(&active).unwrap();

    // Act
    let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

    // Assert
    assert!(error.to_string().contains("no x-workflow metadata"));
}

#[test]
fn closes_containers_bottom_up_after_tasks_archive() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let third_id = item_id(&instance, "third");
    let story_id = item_id(&instance, "story");
    let store = Store::new(project.project_dir());
    assert!(project.run_tg(&["done", &first_id]).status.success());
    assert!(project.run_tg(&["done", &second_id]).status.success());
    assert!(project.run_tg(&["done", &third_id]).status.success());

    // Act
    let outcome = run_shell(&project, &campaign_id);

    // Assert
    assert_eq!(outcome.outcome, WorkflowRunKind::Complete);
    let archive = store.load_all_archive_strict().unwrap();
    let archived_ids = archive
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    let story_index = archived_ids.iter().position(|id| *id == story_id).unwrap();
    let campaign_index = archived_ids
        .iter()
        .position(|id| *id == campaign_id)
        .unwrap();
    assert!(
        story_index < campaign_index,
        "containers should close bottom-up"
    );
    assert!(store.load_active().unwrap().is_empty());
}

#[test]
fn rejects_a_second_campaign_runner_without_blocking_another_campaign() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let first = instantiate(&project, "ONE");
    let second = instantiate(&project, "TWO");
    let first_campaign = item_id(&first, "campaign");
    let second_campaign = item_id(&second, "campaign");
    let results_dir = project.project_dir().join("workflow-results");
    fs::create_dir_all(&results_dir).unwrap();
    let before = workflow_bytes(&project);
    let lock_path = campaign_lock_path(&results_dir, &first_campaign);
    let mut holder = Command::new("python3")
        .args([
            "-c",
            "import fcntl, sys, time; f = open(sys.argv[1], 'a+'); fcntl.flock(f, fcntl.LOCK_EX); print('locked', flush=True); time.sleep(10)",
        ])
        .arg(lock_path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = [0; 7];
    holder
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut ready)
        .unwrap();
    assert_eq!(&ready, b"locked\n");

    // Act
    let blocked = project.run_tg(&["workflow", "run", &first_campaign]);
    let after_rejection = workflow_bytes(&project);
    let other = project.run_tg(&["--json", "workflow", "run", &second_campaign]);
    holder.kill().unwrap();
    holder.wait().unwrap();

    // Assert
    assert_eq!(blocked.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("Lock already held"));
    assert_eq!(after_rejection, before);
    assert!(
        other.status.success(),
        "{}",
        String::from_utf8_lossy(&other.stderr)
    );
    let outcome: serde_json::Value = serde_json::from_slice(&other.stdout).unwrap();
    assert_eq!(outcome["campaign_id"], second_campaign);
    assert_eq!(outcome["outcome"], "claimed");
}

#[test]
fn rejects_archived_containers_with_active_descendants_without_mutation() {
    for container_node in ["story", "campaign"] {
        // Arrange
        let project = TestProject::new().unwrap();
        write_fixture(&project);
        let instance = instantiate(&project, "ONE");
        let campaign_id = item_id(&instance, "campaign");
        let container_id = item_id(&instance, container_node);
        let store = Store::new(project.project_dir());
        archive_item(&store, &container_id);
        let before = workflow_bytes(&project);

        // Act
        let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

        // Assert
        assert!(error.to_string().contains("active descendant"));
        assert_eq!(workflow_bytes(&project), before, "{container_node}");
    }
}

#[test]
fn rejects_single_doing_tasks_with_malformed_claims_without_mutation() {
    for malformed_claim in ["missing", "wrong", "missing_at"] {
        // Arrange
        let project = TestProject::new().unwrap();
        write_fixture(&project);
        let instance = instantiate(&project, "ONE");
        let campaign_id = item_id(&instance, "campaign");
        let first_id = item_id(&instance, "first");
        let store = Store::new(project.project_dir());
        let mut active = store.load_active().unwrap();
        let task = active.iter_mut().find(|item| item.id == first_id).unwrap();
        task.status = Status::Doing;
        match malformed_claim {
            "missing" => {}
            "wrong" => {
                task.claimed_by = Some("workflow:other".to_string());
                task.claimed_at = Some(Utc::now());
            }
            "missing_at" => {
                task.claimed_by = Some(format!("workflow:{campaign_id}"));
            }
            _ => unreachable!(),
        }
        store.save_active(&active).unwrap();
        let before = workflow_bytes(&project);

        // Act
        let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

        // Assert
        assert!(error.to_string().contains("ambiguous doing Tasks"));
        assert_eq!(workflow_bytes(&project), before, "{malformed_claim}");
    }
}

#[test]
fn orders_ready_tasks_by_priority_then_creation_time_then_id() {
    for ordering in ["priority", "created_at", "id"] {
        // Arrange
        let project = TestProject::new().unwrap();
        write_fixture(&project);
        let instance = instantiate(&project, "ONE");
        let campaign_id = item_id(&instance, "campaign");
        let first_id = item_id(&instance, "first");
        let third_id = item_id(&instance, "third");
        let store = Store::new(project.project_dir());
        let mut active = store.load_active().unwrap();
        let first_index = active.iter().position(|item| item.id == first_id).unwrap();
        let third_index = active.iter().position(|item| item.id == third_id).unwrap();
        let timestamp = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let expected = match ordering {
            "priority" => {
                active[first_index].priority = 1;
                active[third_index].priority = 2;
                third_id.as_str()
            }
            "created_at" => {
                active[first_index].priority = 2;
                active[third_index].priority = 2;
                active[first_index].created_at = timestamp;
                active[third_index].created_at = timestamp + chrono::Duration::seconds(1);
                first_id.as_str()
            }
            "id" => {
                active[first_index].priority = 2;
                active[third_index].priority = 2;
                active[first_index].created_at = timestamp;
                active[third_index].created_at = timestamp;
                [first_id.as_str(), third_id.as_str()]
                    .into_iter()
                    .min()
                    .unwrap()
            }
            _ => unreachable!(),
        };
        store.save_active(&active).unwrap();

        // Act
        let outcome = run_shell(&project, &campaign_id);

        // Assert
        assert_eq!(outcome.task_id.as_deref(), Some(expected), "{ordering}");
    }
}

#[test]
fn incomplete_task_descendants_prevent_container_rollup() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let story_id = item_id(&instance, "story");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let store = Store::new(project.project_dir());
    archive_item(&store, &first_id);
    archive_item(&store, &second_id);

    // Act
    let outcome = run_shell(&project, &campaign_id);

    // Assert
    assert_eq!(outcome.outcome, WorkflowRunKind::Claimed);
    let active_ids = store
        .load_active()
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    assert!(active_ids.contains(&story_id));
    assert!(active_ids.contains(&campaign_id));
}

#[test]
fn archived_dependencies_unlock_downstream_tasks() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let store = Store::new(project.project_dir());
    archive_item(&store, &first_id);
    let mut active = store.load_active().unwrap();
    active
        .iter_mut()
        .find(|item| item.id == second_id)
        .unwrap()
        .priority = 1;
    store.save_active(&active).unwrap();

    // Act
    let outcome = run_shell(&project, &campaign_id);

    // Assert
    assert_eq!(outcome.task_id.as_deref(), Some(second_id.as_str()));
}

#[test]
fn all_blocked_campaigns_error_without_mutation() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    for node in ["first", "third"] {
        let item = item_id(&instance, node);
        assert!(project.run_tg(&["block", &item]).status.success());
    }
    let store = Store::new(project.project_dir());
    let before = workflow_bytes(&project);

    // Act
    let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

    // Assert
    assert!(error.to_string().contains("no ready Task"));
    assert_eq!(workflow_bytes(&project), before);
}

#[test]
fn rejects_detached_matching_instance_items_without_writes() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let first_id = item_id(&instance, "first");
    let store = Store::new(project.project_dir());
    let mut active = store.load_active().unwrap();
    let mut detached = active
        .iter()
        .find(|item| item.id == first_id)
        .unwrap()
        .clone();
    detached.id = "detached".to_string();
    detached.parent = None;
    active.push(detached);
    store.save_active(&active).unwrap();
    let before = workflow_bytes(&project);

    // Act
    let error = run_campaign_state_shell(&store, &campaign_id).unwrap_err();

    // Assert
    assert!(error.to_string().contains("outside Campaign"));
    assert_eq!(workflow_bytes(&project), before);
}

#[test]
fn cli_renders_human_success_and_json_errors_with_exit_codes() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");

    // Act
    let success = project.run_tg(&["workflow", "run", &campaign_id]);
    let failure = project.run_tg(&["--json", "workflow", "run", "missing"]);

    // Assert
    assert!(success.status.success());
    assert!(String::from_utf8_lossy(&success.stdout).contains("Outcome: claimed"));
    assert_eq!(failure.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert_eq!(error["exit_code"], 1);
    assert!(error["error"].as_str().unwrap().contains("was not found"));
}

#[test]
fn slash_prefixed_campaigns_use_a_contained_deterministic_process_lock() {
    // Arrange
    let project = TestProject::new().unwrap();
    fs::write(
        project.project_dir().join("config.yaml"),
        "id_prefix: 'team/alpha'\n",
    )
    .unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let results_dir = project.project_dir().join("workflow-results");

    // Act
    let output = project.run_tg(&["workflow", "run", &campaign_id]);

    // Assert
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock_path = campaign_lock_path(&results_dir, &campaign_id);
    assert_eq!(lock_path.parent(), Some(results_dir.as_path()));
    assert!(
        lock_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("campaign-")
    );
    assert!(lock_path.exists());
}

#[test]
fn claim_and_container_completion_events_move_through_existing_logs() {
    // Arrange
    let project = TestProject::new().unwrap();
    write_fixture(&project);
    let instance = instantiate(&project, "ONE");
    let campaign_id = item_id(&instance, "campaign");
    let story_id = item_id(&instance, "story");
    let first_id = item_id(&instance, "first");
    let second_id = item_id(&instance, "second");
    let third_id = item_id(&instance, "third");
    let store = Store::new(project.project_dir());
    let claimed = run_shell(&project, &campaign_id);
    assert_eq!(claimed.outcome, WorkflowRunKind::Claimed);
    let active_events = fs::read_to_string(store.events_path()).unwrap();

    // Act
    for task_id in [&first_id, &second_id, &third_id] {
        assert!(project.run_tg(&["done", task_id]).status.success());
    }
    assert_eq!(
        run_shell(&project, &campaign_id).outcome,
        WorkflowRunKind::Complete
    );

    // Assert
    assert!(active_events.contains(claimed.task_id.as_deref().unwrap()));
    assert!(active_events.contains("\"status\":\"doing\""));
    let archived_events = fs::read_to_string(store.events_archive_path()).unwrap();
    for item_id in [&story_id, &campaign_id] {
        assert!(archived_events.contains(item_id));
    }
    assert!(archived_events.contains("\"status\":\"done\""));
}
