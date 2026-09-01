mod common;

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::process::Stdio;

use common::TestProject;
use task_golem::cache;
use task_golem::errors::TgError;
use task_golem::model::graph::{GraphApplyCategory, GraphApplyDiagnosticCode, GraphApplyError};
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::{GraphApplyItem, GraphApplyRequest, GraphRef};

const ACTIVE_ANCHOR_ID: &str = "018f2b1c-4d5e-7abc-8123-456789abcdef";
const ARCHIVED_ANCHOR_ID: &str = "018f2b1c-4d5e-7abc-9234-56789abcdef0";
const ABSENT_ID: &str = "018f2b1c-4d5e-7abc-a345-6789abcdef01";

fn item(id: &str, title: &str, status: Status) -> Item {
    let now = chrono::Utc::now();
    Item {
        id: id.to_string(),
        title: title.to_string(),
        status,
        priority: 0,
        description: None,
        tags: vec![],
        dependencies: vec![],
        created_at: now,
        updated_at: now,
        blocked_reason: None,
        blocked_from_status: None,
        claimed_by: None,
        claimed_at: None,
        parent: None,
        extensions: BTreeMap::new(),
    }
}

fn graph_item(reference: &str, title: &str) -> GraphApplyItem {
    GraphApplyItem {
        reference: reference.to_string(),
        title: title.to_string(),
        description: None,
        priority: 0,
        tags: vec![],
        parent: None,
        dependencies: vec![],
        extensions: BTreeMap::new(),
    }
}

fn graph_request(items: Vec<GraphApplyItem>) -> GraphApplyRequest {
    GraphApplyRequest { items }
}

fn graph_error(error: TgError) -> GraphApplyError {
    match error {
        TgError::GraphApply(error) => error,
        other => panic!("expected graph_apply error, got {other:?}"),
    }
}

fn initialized_store() -> (tempfile::TempDir, Store) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let store = Store::new(temporary_directory.path().to_path_buf());
    store.with_lock(|store| store.save_active(&[])).unwrap();
    (temporary_directory, store)
}

fn read_or_empty(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

fn durable_state(store: &Store) -> [Vec<u8>; 5] {
    [
        read_or_empty(&store.tasks_path()),
        read_or_empty(&store.archive_path()),
        read_or_empty(&store.events_path()),
        read_or_empty(&store.events_archive_path()),
        read_or_empty(&store.cache_db_path()),
    ]
}

#[test]
fn apply_graph_creates_complete_graph_without_mutating_anchors_or_other_stores() {
    // Arrange
    let (_temporary_directory, store) = initialized_store();
    let active_anchor = item(ACTIVE_ANCHOR_ID, "Active anchor", Status::Todo);
    let archived_anchor = item(ARCHIVED_ANCHOR_ID, "Archived anchor", Status::Done);
    store
        .with_lock(|store| store.save_active(std::slice::from_ref(&active_anchor)))
        .unwrap();
    store.append_to_archive(&archived_anchor).unwrap();
    let cache_before = cache::open_or_rebuild(&store, false).unwrap();
    assert_eq!(
        cache_before
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let archive_before = std::fs::read(store.archive_path()).unwrap();
    let mut root = graph_item("root", "Root");
    root.priority = 7;
    root.tags = vec!["workflow".to_string(), "root".to_string()];
    root.parent = Some(GraphRef::Existing(ACTIVE_ANCHOR_ID.to_string()));
    root.dependencies = vec![GraphRef::Existing(ARCHIVED_ANCHOR_ID.to_string())];
    root.extensions
        .insert("x-owner".to_string(), serde_json::json!({"team": "pg"}));
    let mut child = graph_item("child", "Child");
    child.description = Some("A child task".to_string());
    child.parent = Some(GraphRef::Local("root".to_string()));
    child.dependencies = vec![GraphRef::Local("root".to_string())];

    // Act
    let result = store.apply_graph(graph_request(vec![child, root])).unwrap();

    // Assert
    assert_eq!(result.count, 2);
    assert_eq!(
        result.mapping.keys().collect::<Vec<_>>(),
        vec![&"child".to_string(), &"root".to_string()]
    );
    assert!(
        result
            .mapping
            .values()
            .all(|item_id| task_golem::validate_id(item_id).is_ok())
    );
    let stored = store.load_active().unwrap();
    assert_eq!(
        stored
            .iter()
            .find(|stored_item| stored_item.id == ACTIVE_ANCHOR_ID),
        Some(&active_anchor)
    );
    let stored_root = stored
        .iter()
        .find(|stored_item| stored_item.id == result.mapping["root"])
        .unwrap();
    let stored_child = stored
        .iter()
        .find(|stored_item| stored_item.id == result.mapping["child"])
        .unwrap();
    assert_eq!(stored_root.parent.as_deref(), Some(ACTIVE_ANCHOR_ID));
    assert_eq!(stored_root.dependencies, vec![ARCHIVED_ANCHOR_ID]);
    assert_eq!(stored_root.tags, vec!["workflow", "root"]);
    assert_eq!(stored_root.extensions["x-owner"]["team"], "pg");
    assert_eq!(
        stored_child.parent.as_deref(),
        Some(result.mapping["root"].as_str())
    );
    assert_eq!(
        stored_child.dependencies,
        vec![result.mapping["root"].clone()]
    );
    assert!([stored_root, stored_child].iter().all(|stored_item| {
        stored_item.status == Status::Todo
            && stored_item.claimed_by.is_none()
            && stored_item.claimed_at.is_none()
            && stored_item.blocked_reason.is_none()
            && stored_item.blocked_from_status.is_none()
            && stored_item.created_at == stored_item.updated_at
    }));
    assert_eq!(stored_root.created_at, stored_child.created_at);
    assert_eq!(std::fs::read(store.archive_path()).unwrap(), archive_before);
    assert!(!store.events_path().exists());
    assert!(!store.events_archive_path().exists());
    assert_eq!(
        cache_before
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1,
        "graph apply must not mutate the derived cache directly"
    );
    let rebuilt_cache = cache::open_or_rebuild(&store, false).unwrap();
    assert_eq!(
        rebuilt_cache
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn identical_graph_requests_create_disjoint_fresh_graphs() {
    // Arrange
    let (_temporary_directory, store) = initialized_store();
    let request = graph_request(vec![graph_item("root", "Root")]);

    // Act
    let first = store.apply_graph(request.clone()).unwrap();
    let second = store.apply_graph(request).unwrap();

    // Assert
    assert_ne!(first.mapping, second.mapping);
    assert!(
        first
            .mapping
            .values()
            .collect::<HashSet<_>>()
            .is_disjoint(&second.mapping.values().collect::<HashSet<_>>())
    );
    assert_eq!(store.load_active().unwrap().len(), 2);
}

#[test]
fn invalid_graph_requests_return_ordered_diagnostics_without_durable_changes() {
    let mut duplicate_reference_second = graph_item("same", "Second");
    duplicate_reference_second.dependencies = vec![GraphRef::Existing("invalid-id".to_string())];
    let mut missing_reference = graph_item("missing", "Missing");
    missing_reference.dependencies = vec![GraphRef::Local("absent".to_string())];
    let mut invalid_id = graph_item("invalid-id", "Invalid ID");
    invalid_id.dependencies = vec![GraphRef::Existing("invalid-id".to_string())];
    let invalid_item = graph_item("invalid-item", "\n");
    let mut self_reference = graph_item("self", "Self");
    self_reference.parent = Some(GraphRef::Local("self".to_string()));
    let mut duplicate_dependency = graph_item("duplicate-dependency", "Duplicate dependency");
    duplicate_dependency.dependencies = vec![
        GraphRef::Existing(ACTIVE_ANCHOR_ID.to_string()),
        GraphRef::Existing(ACTIVE_ANCHOR_ID.to_string()),
    ];
    let mut parent_a = graph_item("parent-a", "Parent A");
    parent_a.parent = Some(GraphRef::Local("parent-b".to_string()));
    let mut parent_b = graph_item("parent-b", "Parent B");
    parent_b.parent = Some(GraphRef::Local("parent-a".to_string()));
    let mut dependency_a = graph_item("dependency-a", "Dependency A");
    dependency_a.dependencies = vec![GraphRef::Local("dependency-b".to_string())];
    let mut dependency_b = graph_item("dependency-b", "Dependency B");
    dependency_b.dependencies = vec![GraphRef::Local("dependency-a".to_string())];

    let cases = [
        (
            graph_request(vec![
                graph_item("same", "First"),
                duplicate_reference_second,
            ]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::DuplicateReference],
        ),
        (
            graph_request(vec![invalid_id]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::InvalidId],
        ),
        (
            graph_request(vec![invalid_item]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::InvalidItem],
        ),
        (
            graph_request(vec![missing_reference]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::MissingReference],
        ),
        (
            graph_request(vec![self_reference]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::SelfReference],
        ),
        (
            graph_request(vec![duplicate_dependency]),
            GraphApplyCategory::InvalidRequest,
            vec![GraphApplyDiagnosticCode::DuplicateDependency],
        ),
        (
            graph_request(vec![parent_a, parent_b]),
            GraphApplyCategory::InvalidGraph,
            vec![GraphApplyDiagnosticCode::ParentCycle],
        ),
        (
            graph_request(vec![dependency_a, dependency_b]),
            GraphApplyCategory::InvalidGraph,
            vec![GraphApplyDiagnosticCode::DependencyCycle],
        ),
    ];

    for (request, expected_category, expected_codes) in cases {
        // Arrange
        let (_temporary_directory, store) = initialized_store();
        store
            .with_lock(|store| store.save_active(&[item(ACTIVE_ANCHOR_ID, "Anchor", Status::Todo)]))
            .unwrap();
        cache::open_or_rebuild(&store, false).unwrap();
        let state_before = durable_state(&store);

        // Act
        let error = graph_error(store.apply_graph(request).unwrap_err());

        // Assert
        assert_eq!(error.category, expected_category);
        assert_eq!(
            error
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            expected_codes
        );
        assert!(
            error
                .diagnostics
                .windows(2)
                .all(|pair| pair[0].path <= pair[1].path)
        );
        assert_eq!(durable_state(&store), state_before);
    }
}

#[test]
fn invalid_durable_anchor_returns_storage_corruption_without_writing() {
    // Arrange
    let (_temporary_directory, store) = initialized_store();
    let duplicate = item(ACTIVE_ANCHOR_ID, "Duplicate", Status::Todo);
    let line = serde_json::to_string(&duplicate).unwrap();
    std::fs::write(
        store.tasks_path(),
        format!("{{\"schema_version\":1}}\n{line}\n{line}\n"),
    )
    .unwrap();
    let tasks_before = std::fs::read(store.tasks_path()).unwrap();

    // Act
    let error = graph_error(
        store
            .apply_graph(graph_request(vec![graph_item("root", "Root")]))
            .unwrap_err(),
    );

    // Assert
    assert_eq!(error.category, GraphApplyCategory::StorageCorruption);
    assert!(
        error
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == GraphApplyDiagnosticCode::DuplicateDurableId)
    );
    assert_eq!(std::fs::read(store.tasks_path()).unwrap(), tasks_before);
}

#[test]
fn noncanonical_archived_anchor_returns_storage_corruption_without_durable_changes() {
    // Arrange
    let (_temporary_directory, store) = initialized_store();
    let archived_anchor = item("tg-legacy", "Archived anchor", Status::Done);
    let archived_line = serde_json::to_string(&archived_anchor).unwrap();
    std::fs::write(
        store.archive_path(),
        format!("{{\"schema_version\":1}}\n{archived_line}\n"),
    )
    .unwrap();
    let state_before = durable_state(&store);

    // Act
    let error = graph_error(
        store
            .apply_graph(graph_request(vec![graph_item("root", "Root")]))
            .unwrap_err(),
    );

    // Assert
    assert_eq!(error.category, GraphApplyCategory::StorageCorruption);
    assert_eq!(
        error.diagnostics[0].code,
        GraphApplyDiagnosticCode::InvalidId
    );
    assert_eq!(durable_state(&store), state_before);
}

#[test]
fn cli_apply_emits_the_public_result_and_error_envelopes() {
    // Arrange
    let project = TestProject::new().unwrap();
    let request = graph_request(vec![graph_item("root", "Root")]);

    // Act
    let success = run_apply_cli(&project, &serde_json::to_value(request).unwrap());
    let invalid_request = serde_json::json!({
        "items": [{
            "ref": "root",
            "id": ABSENT_ID,
            "title": "Root",
            "description": null,
            "priority": 0,
            "tags": [],
            "parent": null,
            "dependencies": [],
            "extensions": {}
        }]
    });
    let failure = run_apply_cli(&project, &invalid_request);

    // Assert
    assert!(success.status.success());
    let success_json: serde_json::Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(success_json["operation"], "graph_apply");
    assert_eq!(success_json["outcome"], "created");
    assert_eq!(success_json["count"], 1);
    task_golem::validate_id(success_json["mapping"]["root"].as_str().unwrap()).unwrap();
    assert_eq!(success_json.as_object().unwrap().len(), 4);

    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    let failure_json: serde_json::Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert_eq!(failure_json["operation"], "graph_apply");
    assert_eq!(failure_json["outcome"], "error");
    assert_eq!(failure_json["category"], "invalid_request");
    assert_eq!(failure_json["diagnostics"][0]["code"], "invalid_item");
    assert_eq!(failure_json["exit_code"], 1);
    assert_eq!(failure_json.as_object().unwrap().len(), 5);
}

fn run_apply_cli(project: &TestProject, request: &serde_json::Value) -> std::process::Output {
    let mut child = project
        .tg_cmd()
        .args(["--json", "apply"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(request).unwrap().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
