use std::fmt::Write;
use std::fs;
use std::path::Path;

use task_golem::workflow::template::{
    ContextMode, NodeKind, PluginResultStatus, load_workflow_definition, parse_plugin_request,
    parse_plugin_result,
};

fn write_project_file(workspace: &Path, relative_path: &str, contents: &str) {
    let path = workspace.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn valid_template() -> &'static str {
    r#"
version: 1
name: example
plugins:
  writer: .task-golem/plugins/writer.yaml
nodes:
  - id: campaign
    kind: container
    title: Campaign
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
      change_path: changes/example/CHANGE.md
    verify: ["just", "check"]
  - id: review
    kind: task
    parent: story
    depends_on: [write]
    title: Review
    plugin: writer
    context: fresh
"#
}

fn load_template(
    template: &str,
    inputs: &[&str],
) -> Result<task_golem::workflow::template::WorkflowDefinition, task_golem::errors::TgError> {
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    let project_dir = workspace.join(".task-golem");
    fs::create_dir_all(&project_dir).unwrap();
    write_project_file(
        workspace,
        ".task-golem/plugins/writer.yaml",
        "version: 1\nargv: [\"python3\", \"scripts/writer.py\"]\n",
    );
    write_project_file(workspace, ".task-golem/workflows/example.yaml", template);

    load_workflow_definition(
        &project_dir,
        Path::new(".task-golem/workflows/example.yaml"),
        &inputs
            .iter()
            .map(|input| input.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn loads_resolved_contract_and_leaves_argv_literal() {
    // Arrange
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path();
    let project_dir = workspace.join(".task-golem");
    fs::create_dir_all(&project_dir).unwrap();
    write_project_file(
        workspace,
        ".task-golem/plugins/writer.yaml",
        "version: 1\nargv: [\"${plugin_command}\", \"\", \"   \"]\n",
    );
    write_project_file(
        workspace,
        ".task-golem/workflows/example.yaml",
        &valid_template().replace(
            "verify: [\"just\", \"check\"]",
            "verify: [\"just\", \"\", \"   \"]",
        ),
    );

    // Act
    let definition = load_workflow_definition(
        &project_dir,
        Path::new(".task-golem/workflows/example.yaml"),
        &[],
    )
    .unwrap();

    // Assert
    assert_eq!(definition.version, 1);
    assert_eq!(definition.name, "example");
    assert!(definition.template_path.is_absolute());
    assert_eq!(
        definition.plugins["writer"].argv,
        ["${plugin_command}", "", "   "]
    );
    assert!(definition.plugins["writer"].definition_path.is_absolute());
    let write = definition
        .nodes
        .iter()
        .find(|node| node.id == "write")
        .unwrap();
    assert_eq!(write.kind, NodeKind::Task);
    assert_eq!(
        write.verify.as_deref(),
        Some(&["just".into(), "".into(), "   ".into()][..])
    );
    assert_eq!(write.context.as_ref().unwrap().mode, ContextMode::Shared);
    assert_eq!(
        write.context.as_ref().unwrap().key.as_deref(),
        Some("story")
    );
}

#[test]
fn substitutes_exact_non_verify_scalars_and_requires_exact_input_set() {
    // Arrange
    let template = valid_template()
        .replace("title: Write", "title: ${title}")
        .replace(
            "change_path: changes/example/CHANGE.md",
            "${input_key}: ${change_path}\n      literal: prefix-${title}\n      nested:\n        verify: [\"${nested_value}\"]",
        )
        .replace(
            "verify: [\"just\", \"check\"]",
            "verify: [\"${title}\", \"check\"]",
        );

    // Act
    let definition = load_template(
        &template,
        &[
            "title=Implement",
            "input_key=change_path",
            "change_path=changes/WRK-1/CHANGE.md",
            "nested_value=replaced",
        ],
    )
    .unwrap();

    // Assert
    let write = definition
        .nodes
        .iter()
        .find(|node| node.id == "write")
        .unwrap();
    assert_eq!(write.title, "Implement");
    assert_eq!(
        write.input["change_path"],
        serde_json::json!("changes/WRK-1/CHANGE.md")
    );
    assert_eq!(write.input["literal"], serde_json::json!("prefix-${title}"));
    assert_eq!(
        write.input["nested"]["verify"],
        serde_json::json!(["replaced"])
    );
    assert_eq!(
        write.verify.as_deref(),
        Some(&["${title}".into(), "check".into()][..])
    );

    for (inputs, expected) in [
        (
            vec![
                "title=Implement",
                "input_key=change_path",
                "nested_value=replaced",
            ],
            "missing workflow inputs: change_path",
        ),
        (
            vec![
                "title=Implement",
                "input_key=change_path",
                "change_path=changes/X",
                "nested_value=replaced",
                "unused=value",
            ],
            "unexpected workflow inputs: unused",
        ),
        (
            vec![
                "title=One",
                "title=Two",
                "input_key=change_path",
                "change_path=changes/X",
                "nested_value=replaced",
            ],
            "duplicate workflow input 'title'",
        ),
    ] {
        let error = load_template(&template, &inputs).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}', got '{error}'"
        );
    }
}

#[test]
fn rejects_mapping_key_collisions_after_input_substitution() {
    // Arrange
    let template = valid_template().replace(
        "change_path: changes/example/CHANGE.md",
        "existing: one\n      ${input_key}: two",
    );

    // Act
    let error = load_template(&template, &["input_key=existing"]).unwrap_err();

    // Assert
    assert!(
        error
            .to_string()
            .contains("duplicate mapping key 'existing'"),
        "got '{error}'"
    );
}

#[test]
fn rejects_malformed_template_graph_and_context_fields() {
    let cases = [
        (
            "unsupported version",
            valid_template().replace("version: 1", "version: 2"),
            "workflow template version must be 1",
        ),
        (
            "unknown field",
            valid_template().replace("name: example", "name: example\nunknown: value"),
            "unknown field",
        ),
        (
            "duplicate node",
            valid_template().replace("id: review", "id: write"),
            "duplicate workflow node id 'write'",
        ),
        (
            "multiline title",
            valid_template().replace("title: Story", "title: \"Story\\ncontinued\""),
            "Title must be a single line",
        ),
        (
            "multiple roots",
            valid_template().replace("    parent: campaign\n    title: Story", "    title: Story"),
            "exactly one root container",
        ),
        (
            "root task",
            valid_template().replace("    parent: story\n    depends_on", "    depends_on"),
            "exactly one root container",
        ),
        (
            "dangling parent",
            valid_template().replace("parent: story", "parent: missing"),
            "references missing parent 'missing'",
        ),
        (
            "task parent",
            valid_template().replace("parent: story\n    depends_on", "parent: write\n    depends_on"),
            "parent 'write' is not a container",
        ),
        (
            "parent cycle",
            valid_template().replace("    title: Campaign", "    parent: story\n    title: Campaign"),
            "parent cycle",
        ),
        (
            "container plugin",
            valid_template().replace("    title: Story", "    title: Story\n    plugin: writer"),
            "container 'story' cannot declare plugin",
        ),
        (
            "container input",
            valid_template().replace("    title: Story", "    title: Story\n    input: {}"),
            "container 'story' cannot declare input",
        ),
        (
            "task missing plugin",
            valid_template().replace("    plugin: writer\n    context: fresh", "    context: fresh"),
            "task 'review' must declare plugin",
        ),
        (
            "task missing context",
            valid_template().replace("    context: fresh", ""),
            "task 'review' must declare context",
        ),
        (
            "invalid context",
            valid_template().replace("context: fresh", "context: session"),
            "invalid workflow context 'session'",
        ),
        (
            "missing plugin definition",
            valid_template().replace("plugin: writer", "plugin: absent"),
            "references unknown plugin 'absent'",
        ),
        (
            "container dependency",
            valid_template().replace("    title: Story", "    title: Story\n    depends_on: [write]"),
            "container 'story' cannot declare depends_on",
        ),
        (
            "dependency target container",
            valid_template().replace("depends_on: [write]", "depends_on: [story]"),
            "dependency 'story' is not a task",
        ),
        (
            "dependency cycle",
            valid_template().replace("    title: Write", "    depends_on: [review]\n    title: Write"),
            "dependency cycle",
        ),
        (
            "duplicate dependency",
            valid_template().replace("depends_on: [write]", "depends_on: [write, write]"),
            "duplicate dependency 'write'",
        ),
        (
            "missing dependency",
            valid_template().replace("depends_on: [write]", "depends_on: [missing]"),
            "references missing dependency 'missing'",
        ),
        (
            "shared target is not ancestor",
            valid_template().replace("context: shared:story", "context: shared:campaign-missing"),
            "shared context 'campaign-missing' is not an ancestor container",
        ),
        (
            "shared plugin mismatch",
            valid_template()
                .replace(
                    "writer: .task-golem/plugins/writer.yaml",
                    "writer: .task-golem/plugins/writer.yaml\n  reviewer: .task-golem/plugins/writer.yaml",
                )
                .replace("plugin: writer\n    context: fresh", "plugin: reviewer\n    context: shared:story"),
            "shared context 'story' uses multiple plugins",
        ),
        (
            "empty verify",
            valid_template().replace("verify: [\"just\", \"check\"]", "verify: []"),
            "task 'write' verify argv cannot be empty",
        ),
        (
            "blank verify program",
            valid_template().replace("verify: [\"just\", \"check\"]", "verify: [\"   \"]"),
            "task 'write' verify argv program cannot be empty",
        ),
        (
            "NUL verify program",
            valid_template().replace("verify: [\"just\", \"check\"]", r#"verify: ["\0"]"#),
            "task 'write' verify argv contains an argument with NUL",
        ),
        (
            "NUL verify argument",
            valid_template().replace(
                "verify: [\"just\", \"check\"]",
                r#"verify: ["just", "bad\0argument"]"#,
            ),
            "task 'write' verify argv contains an argument with NUL",
        ),
        (
            "container without executable descendant",
            valid_template().replace(
                "  - id: story",
                "  - id: empty\n    kind: container\n    parent: campaign\n    title: Empty\n  - id: story",
            ),
            "container 'empty' has no executable descendant",
        ),
    ];

    for (name, template, expected) in cases {
        let error = load_template(&template, &[]).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "case '{name}' expected '{expected}', got '{error}'"
        );
    }
}

#[test]
fn validates_a_deep_hierarchy_with_many_shared_context_tasks() {
    // Arrange
    const CONTAINER_COUNT: usize = 25_000;
    const TASK_COUNT: usize = 25_000;
    const NODE_COUNT: usize = CONTAINER_COUNT + TASK_COUNT;
    let mut template = String::with_capacity(NODE_COUNT * 100);
    template.push_str(
        "version: 1\nname: deep\nplugins:\n  writer: .task-golem/plugins/writer.yaml\nnodes:\n",
    );
    template.push_str("  - id: node-0\n    kind: container\n    title: Node 0\n");
    for index in 1..CONTAINER_COUNT {
        writeln!(
            template,
            "  - id: node-{index}\n    kind: container\n    parent: node-{}\n    title: Node {index}",
            index - 1
        )
        .unwrap();
    }
    for index in 0..TASK_COUNT {
        writeln!(
            template,
            "  - id: task-{index}\n    kind: task\n    parent: node-{}\n    title: Task {index}\n    plugin: writer\n    context: shared:node-{}",
            CONTAINER_COUNT - 1,
            CONTAINER_COUNT - 1
        )
        .unwrap();
    }

    // Act
    let definition = load_template(&template, &[]).unwrap();

    // Assert
    assert_eq!(definition.nodes.len(), NODE_COUNT);
}

#[test]
fn rejects_definition_paths_outside_workspace_and_malformed_plugins() {
    // Arrange
    let temp_dir = tempfile::tempdir().unwrap();
    let workspace = temp_dir.path().join("workspace");
    let project_dir = workspace.join(".task-golem");
    fs::create_dir_all(&project_dir).unwrap();
    let outside_template = temp_dir.path().join("outside-template.yaml");
    fs::write(&outside_template, valid_template()).unwrap();

    // Act
    let template_error =
        load_workflow_definition(&project_dir, &outside_template, &[]).unwrap_err();

    // Assert
    assert!(
        template_error
            .to_string()
            .contains("outside workspace root")
    );

    for (plugin, expected) in [
        (
            "version: 2\nargv: [\"agent\"]\n",
            "plugin version must be 1",
        ),
        ("version: 1\nargv: []\n", "plugin argv cannot be empty"),
        (
            "version: 1\nargv: [\"   \"]\n",
            "plugin argv program cannot be empty",
        ),
        (
            r#"version: 1
argv: ["\0"]
"#,
            "plugin argv contains an argument with NUL",
        ),
        (
            r#"version: 1
argv: ["agent", "bad\0argument"]
"#,
            "plugin argv contains an argument with NUL",
        ),
        (
            "version: 1\nargv: [\"agent\"]\nunknown: true\n",
            "unknown field",
        ),
    ] {
        write_project_file(&workspace, ".task-golem/plugins/writer.yaml", plugin);
        write_project_file(
            &workspace,
            ".task-golem/workflows/example.yaml",
            valid_template(),
        );
        let error = load_workflow_definition(
            &project_dir,
            Path::new(".task-golem/workflows/example.yaml"),
            &[],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}', got '{error}'"
        );
    }

    fs::write(
        temp_dir.path().join("outside-plugin.yaml"),
        "version: 1\nargv: [\"agent\"]\n",
    )
    .unwrap();
    write_project_file(
        &workspace,
        ".task-golem/workflows/example.yaml",
        &valid_template().replace(".task-golem/plugins/writer.yaml", "../outside-plugin.yaml"),
    );
    let plugin_error = load_workflow_definition(
        &project_dir,
        Path::new(".task-golem/workflows/example.yaml"),
        &[],
    )
    .unwrap_err();
    assert!(plugin_error.to_string().contains("outside workspace root"));
}

#[test]
fn parses_strict_plugin_request_and_result_contracts() {
    // Arrange
    let request_json = r#"{
        "version": 1,
        "campaign_id": "tg-campaign",
        "task_id": "tg-task",
        "title": "Write",
        "description": null,
        "workspace": "/tmp/workspace",
        "input": {"change_path": "changes/X/CHANGE.md"},
        "context": {
            "mode": "shared",
            "key": "story",
            "session_ref": "session-1",
            "resume": true
        }
    }"#;
    let result_json = r#"{
        "version": 1,
        "status": "complete",
        "summary": "Implemented and checked",
        "session_ref": "session-2"
    }"#;
    let none_request_json = request_json
        .replace("\"mode\": \"shared\"", "\"mode\": \"none\"")
        .replace("\"key\": \"story\"", "\"key\": null")
        .replace("\"session_ref\": \"session-1\"", "\"session_ref\": null")
        .replace("\"resume\": true", "\"resume\": false");

    // Act
    let request = parse_plugin_request(request_json).unwrap();
    let none_request = parse_plugin_request(&none_request_json).unwrap();
    let result = parse_plugin_result(result_json).unwrap();

    // Assert
    assert_eq!(request.context.mode, ContextMode::Shared);
    assert!(request.context.resume);
    assert_eq!(none_request.context.mode, ContextMode::None);
    assert_eq!(none_request.context.key, None);
    assert_eq!(none_request.context.session_ref, None);
    assert!(!none_request.context.resume);
    assert_eq!(result.status, PluginResultStatus::Complete);
    assert_eq!(result.session_ref.as_deref(), Some("session-2"));

    for (json, expected) in [
        (
            request_json.replace("\"version\": 1", "\"version\": 2"),
            "plugin request version must be 1",
        ),
        (
            request_json.replace(
                "\"workspace\": \"/tmp/workspace\"",
                "\"workspace\": \"relative\"",
            ),
            "plugin request workspace must be absolute",
        ),
        (
            request_json.replace("\"key\": \"story\"", "\"key\": null"),
            "shared plugin context requires a key",
        ),
        (
            request_json.replace("\"session_ref\": \"session-1\"", "\"session_ref\": null"),
            "resumed shared plugin context requires a session_ref",
        ),
        (
            request_json.replace("\"mode\": \"shared\"", "\"mode\": \"fresh\""),
            "fresh plugin context cannot include key, session_ref, or resume",
        ),
        (
            request_json.replace(
                "\"title\": \"Write\"",
                "\"title\": \"Write\", \"unknown\": true",
            ),
            "unknown field",
        ),
    ] {
        let error = parse_plugin_request(&json).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}', got '{error}'"
        );
    }

    for (json, expected) in [
        (
            result_json.replace("\"version\": 1", "\"version\": 2"),
            "plugin result version must be 1",
        ),
        (
            result_json.replace("\"complete\"", "\"unknown\""),
            "unknown variant",
        ),
        (
            result_json.replace("Implemented and checked", ""),
            "plugin result summary cannot be empty",
        ),
        (
            result_json.replace("\"summary\":", "\"unknown\": true, \"summary\":"),
            "unknown field",
        ),
    ] {
        let error = parse_plugin_result(&json).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}', got '{error}'"
        );
    }
}
