//! Integration test verifying the library API is usable from an external consumer perspective.

use std::collections::HashSet;

use task_golem::errors::TgError;
use task_golem::generate_id_with_prefix;
use task_golem::git;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;

#[test]
fn store_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();

    let store = Store::new(project_dir);

    // Save empty, then load
    store.with_lock(|s| s.save_active(&[])).unwrap();

    let items = store.load_active().unwrap();
    assert!(items.is_empty());

    // Create and save an item
    let now = chrono::Utc::now();
    let item = Item {
        id: "WRK-a1b2c".to_string(),
        title: "Test item".to_string(),
        status: Status::Todo,
        priority: 0,
        description: Some("A test item".to_string()),
        tags: vec!["test".to_string()],
        dependencies: vec![],
        created_at: now,
        updated_at: now,
        blocked_reason: None,
        blocked_from_status: None,
        claimed_by: None,
        claimed_at: None,
        parent: None,
        extensions: std::collections::BTreeMap::new(),
    };

    store.with_lock(|s| s.save_active(&[item.clone()])).unwrap();

    let loaded = store.load_active().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "WRK-a1b2c");
    assert_eq!(loaded[0].title, "Test item");
    assert_eq!(loaded[0].status, Status::Todo);
    assert_eq!(loaded[0].description.as_deref(), Some("A test item"));
}

#[test]
fn generate_id_with_custom_prefix() {
    let existing = HashSet::new();
    let id = generate_id_with_prefix(&existing, "WRK", 5).unwrap();
    assert!(id.starts_with("WRK-"), "ID should start with WRK-: {}", id);
    assert_eq!(id.len(), 9, "ID should be 9 chars (WRK- + 5): {}", id);
}

#[test]
fn generate_id_with_custom_len() {
    let existing = HashSet::new();
    let id = generate_id_with_prefix(&existing, "WRK", 8).unwrap();
    assert!(id.starts_with("WRK-"), "ID should start with WRK-: {}", id);
    assert_eq!(id.len(), 12, "ID should be 12 chars (WRK- + 8): {}", id);
    // Verify no confusable chars (i, l, o, u)
    let random_part = &id[4..];
    assert!(
        !random_part.contains('i')
            && !random_part.contains('l')
            && !random_part.contains('o')
            && !random_part.contains('u'),
        "Should not contain confusable chars: {}",
        random_part
    );
}

#[test]
fn all_known_ids_includes_active_and_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();

    let store = Store::new(project_dir);
    let now = chrono::Utc::now();

    let active_item = Item {
        id: "WRK-aaaa0".to_string(),
        title: "Active item".to_string(),
        status: Status::Todo,
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
        extensions: std::collections::BTreeMap::new(),
    };

    let archive_item = Item {
        id: "WRK-bbb00".to_string(),
        title: "Archived item".to_string(),
        status: Status::Done,
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
        extensions: std::collections::BTreeMap::new(),
    };

    store.with_lock(|s| s.save_active(&[active_item])).unwrap();

    store.append_to_archive(&archive_item).unwrap();

    let all_ids = store.all_known_ids().unwrap();
    assert!(all_ids.contains("WRK-aaaa0"), "Should contain active ID");
    assert!(all_ids.contains("WRK-bbb00"), "Should contain archive ID");
    assert_eq!(all_ids.len(), 2);
}

#[test]
fn item_apply_unblock_restores_status() {
    let now = chrono::Utc::now();
    let mut item = Item {
        id: "WRK-test1".to_string(),
        title: "Test".to_string(),
        status: Status::Doing,
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
        extensions: std::collections::BTreeMap::new(),
    };

    item.apply_block(Some("blocked reason".to_string()))
        .consume_for_test();
    assert_eq!(item.status, Status::Blocked);
    assert_eq!(item.blocked_from_status, Some(Status::Doing));

    item.apply_unblock().consume_for_test();
    assert_eq!(item.status, Status::Doing);
    assert!(item.blocked_reason.is_none());
    assert!(item.blocked_from_status.is_none());
}

#[test]
fn store_clone_works() {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();

    let store = Store::new(project_dir);
    let store_clone = store.clone();

    // Both should be able to save/load independently
    store.with_lock(|s| s.save_active(&[])).unwrap();

    let items = store_clone.load_active().unwrap();
    assert!(items.is_empty());
}

#[test]
fn item_partial_eq_works() {
    let now = chrono::Utc::now();
    let item1 = Item {
        id: "WRK-test1".to_string(),
        title: "Test".to_string(),
        status: Status::Todo,
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
        extensions: std::collections::BTreeMap::new(),
    };

    let item2 = item1.clone();
    assert_eq!(item1, item2);
}

#[test]
fn error_types_accessible() {
    // Verify TgError variants are constructible from library API
    let err = TgError::ItemNotFound("test".to_string());
    assert_eq!(err.exit_code(), 1);

    let err = TgError::LockTimeout(std::time::Duration::from_secs(5));
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn git_module_accessible() {
    // Verify git functions exist and are callable (they'll fail without a git repo, but we verify the API surface)
    let tmp = tempfile::tempdir().unwrap();
    let result = git::stage_self(tmp.path());
    // Expected to fail since it's not a git repo, but the function is callable
    assert!(result.is_err());
}
