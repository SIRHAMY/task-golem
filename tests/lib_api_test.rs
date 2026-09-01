//! Integration test verifying the library API is usable from an external consumer perspective.

use std::collections::HashSet;

use task_golem::errors::TgError;
use task_golem::git;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::{generate_id, resolve_id, validate_id};

const ACTIVE_ID: &str = "018f2b1c-4d5e-7abc-8123-456789abcdef";
const ARCHIVED_ID: &str = "018f2b1c-4d5e-7abc-9234-56789abcdef0";

fn test_item(id: &str, status: Status) -> Item {
    let now = chrono::Utc::now();
    Item {
        id: id.to_string(),
        title: "Test item".to_string(),
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
        extensions: std::collections::BTreeMap::new(),
    }
}

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
        id: ACTIVE_ID.to_string(),
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
    assert_eq!(loaded[0].id, ACTIVE_ID);
    assert_eq!(loaded[0].title, "Test item");
    assert_eq!(loaded[0].status, Status::Todo);
    assert_eq!(loaded[0].description.as_deref(), Some("A test item"));
}

#[test]
fn canonical_identity_api_is_public() {
    let existing = HashSet::new();
    let id = generate_id(&existing).unwrap();
    validate_id(&id).unwrap();
    assert_eq!(
        resolve_id(&id, std::slice::from_ref(&id), &existing, false).unwrap(),
        id
    );
}

#[test]
fn store_item_lookups_validate_ids() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();
    let store = Store::new(project_dir);
    store.with_lock(|s| s.save_active(&[])).unwrap();

    // Act
    let note_error = store.append_note("tg-legacy", "note").unwrap_err();
    let archive_error = store.load_archive_item("tg-legacy").unwrap_err();
    let absent_error = store.append_note(ACTIVE_ID, "note").unwrap_err();

    // Assert
    assert!(matches!(note_error, TgError::InvalidId(_)));
    assert!(matches!(archive_error, TgError::InvalidId(_)));
    assert!(matches!(absent_error, TgError::ItemNotFound(_)));
}

#[test]
fn store_writes_reject_duplicate_identities_across_active_and_archive() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();
    let store = Store::new(project_dir);
    store.with_lock(|s| s.save_active(&[])).unwrap();
    store
        .append_to_archive(&test_item(ARCHIVED_ID, Status::Done))
        .unwrap();

    // Act
    let result = store.with_lock(|s| s.save_active(&[test_item(ARCHIVED_ID, Status::Todo)]));

    // Assert
    assert!(matches!(result, Err(TgError::StorageCorruption(_))));
    assert!(store.load_active().unwrap().is_empty());
}

#[test]
fn public_archive_append_rejects_active_identity_without_writing() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();
    let store = Store::new(project_dir);
    let item = test_item(ACTIVE_ID, Status::Todo);
    store
        .with_lock(|s| s.save_active(std::slice::from_ref(&item)))
        .unwrap();

    // Act
    let result = store.append_to_archive(&item);

    // Assert
    assert!(matches!(result, Err(TgError::StorageCorruption(_))));
    assert!(store.load_archive_ids().unwrap().is_empty());
}

#[test]
fn status_commit_rejects_invalid_identity_before_event_write() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();
    let store = Store::new(project_dir);
    let mut item = test_item("tg-legacy", Status::Todo);
    let change = item.apply_do(None);

    // Act
    let result = store.with_lock(|s| s.commit_status_change(&[item], change));

    // Assert
    assert!(matches!(result, Err(TgError::InvalidId(_))));
    assert!(!store.events_path().exists());
}

#[test]
fn done_commit_rejects_archive_collision_before_event_write() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&project_dir).unwrap();
    let store = Store::new(project_dir);
    store
        .append_to_archive(&test_item(ARCHIVED_ID, Status::Done))
        .unwrap();
    let archive_before = std::fs::read_to_string(store.archive_path()).unwrap();
    let mut item = test_item(ARCHIVED_ID, Status::Todo);
    let change = item.apply_done();

    // Act
    let result = store.with_lock(|s| s.commit_done(&[], &item, change));

    // Assert
    assert!(matches!(result, Err(TgError::StorageCorruption(_))));
    assert!(!store.events_path().exists());
    assert_eq!(
        std::fs::read_to_string(store.archive_path()).unwrap(),
        archive_before
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
        id: ACTIVE_ID.to_string(),
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
        id: ARCHIVED_ID.to_string(),
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
    assert!(all_ids.contains(ACTIVE_ID), "Should contain active ID");
    assert!(all_ids.contains(ARCHIVED_ID), "Should contain archive ID");
    assert_eq!(all_ids.len(), 2);
}

#[test]
fn item_apply_unblock_restores_status() {
    let now = chrono::Utc::now();
    let mut item = Item {
        id: ACTIVE_ID.to_string(),
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
        id: ACTIVE_ID.to_string(),
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
    assert_eq!(err.code(), "item_not_found");

    let err = TgError::InvalidId("test".to_string());
    assert_eq!(err.exit_code(), 1);
    assert_eq!(err.code(), "invalid_id");

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
