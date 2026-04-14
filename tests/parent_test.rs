//! Phase 1 tests: parent field serde + reparent orchestration.
//!
//! These tests exercise the data-model layer (no CLI). Phase 2 will extend this
//! file with CLI-level tests.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use task_golem::errors::TgError;
use task_golem::model::deps::{
    detect_all_parent_cycles, validate_parent, would_create_parent_cycle,
};
use task_golem::model::item::Item;
use task_golem::model::parent::reparent;
use task_golem::model::status::Status;

fn make_item(id: &str, parent: Option<&str>) -> Item {
    let now: DateTime<Utc> = Utc::now();
    Item {
        id: id.to_string(),
        title: format!("Item {}", id),
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
        parent: parent.map(|s| s.to_string()),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn parent_field_round_trips_some() {
    let item = make_item("tg-aaa00", Some("tg-bbb00"));
    let json = serde_json::to_string(&item).unwrap();
    let back: Item = serde_json::from_str(&json).unwrap();
    assert_eq!(back.parent.as_deref(), Some("tg-bbb00"));
    // Field present and non-null in serialized form.
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["parent"], serde_json::Value::String("tg-bbb00".into()));
}

#[test]
fn parent_field_round_trips_none_as_null() {
    let item = make_item("tg-aaa00", None);
    let json = serde_json::to_string(&item).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["parent"], serde_json::Value::Null);
    let back: Item = serde_json::from_str(&json).unwrap();
    assert_eq!(back.parent, None);
}

#[test]
fn legacy_record_without_parent_deserializes_as_none() {
    // Simulate a record written before `parent` existed: no `parent` key at all.
    let json = r#"{
        "id": "tg-old00",
        "title": "Legacy item",
        "status": "todo",
        "priority": 0,
        "description": null,
        "tags": [],
        "dependencies": [],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "blocked_reason": null,
        "blocked_from_status": null,
        "claimed_by": null,
        "claimed_at": null
    }"#;
    let item: Item = serde_json::from_str(json).unwrap();
    assert_eq!(item.parent, None);
}

#[test]
fn validate_parent_rejects_self() {
    let active: HashSet<String> = ["tg-aaa00".into()].into_iter().collect();
    let archive = HashSet::new();
    let err = validate_parent("tg-aaa00", "tg-aaa00", &active, &archive).unwrap_err();
    assert!(matches!(err, TgError::ParentSelfReference { .. }));
}

#[test]
fn validate_parent_rejects_dangling() {
    let active: HashSet<String> = ["tg-aaa00".into()].into_iter().collect();
    let archive = HashSet::new();
    let err = validate_parent("tg-aaa00", "tg-bogus", &active, &archive).unwrap_err();
    assert!(matches!(err, TgError::ParentDangling { .. }));
}

#[test]
fn validate_parent_rejects_archived_target() {
    let active: HashSet<String> = ["tg-aaa00".into()].into_iter().collect();
    let archive: HashSet<String> = ["tg-old00".into()].into_iter().collect();
    let err = validate_parent("tg-aaa00", "tg-old00", &active, &archive).unwrap_err();
    assert!(matches!(err, TgError::ParentDangling { .. }));
}

#[test]
fn would_create_parent_cycle_direct() {
    let items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", None),
    ];
    // Setting B.parent = A would create A -> B -> A.
    assert!(would_create_parent_cycle(&items, "tg-bbb00", "tg-aaa00"));
}

#[test]
fn would_create_parent_cycle_transitive() {
    let items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", Some("tg-ccc00")),
        make_item("tg-ccc00", None),
    ];
    // Setting C.parent = A would create A -> B -> C -> A.
    assert!(would_create_parent_cycle(&items, "tg-ccc00", "tg-aaa00"));
}

#[test]
fn would_create_parent_cycle_not_a_cycle() {
    let items = vec![
        make_item("tg-aaa00", None),
        make_item("tg-bbb00", None),
        make_item("tg-ccc00", None),
    ];
    assert!(!would_create_parent_cycle(&items, "tg-ccc00", "tg-bbb00"));
}

#[test]
fn detect_all_parent_cycles_none_in_clean_tree() {
    let items = vec![
        make_item("tg-aaa00", None),
        make_item("tg-bbb00", Some("tg-aaa00")),
        make_item("tg-ccc00", Some("tg-aaa00")),
        make_item("tg-ddd00", Some("tg-bbb00")),
    ];
    assert!(detect_all_parent_cycles(&items).is_empty());
}

#[test]
fn detect_all_parent_cycles_finds_cycle() {
    // Hand-crafted cyclic state (bypassing the validator) to confirm detection works.
    let items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", Some("tg-aaa00")),
    ];
    let cycles = detect_all_parent_cycles(&items);
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].len(), 2);
}

#[test]
fn reparent_rejects_direct_cycle() {
    let mut items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", None),
    ];
    let err = reparent(&mut items, "tg-bbb00", Some("tg-aaa00".into()), &[]).unwrap_err();
    assert!(matches!(err, TgError::ParentCycle { .. }));
}

#[test]
fn reparent_rejects_transitive_cycle() {
    let mut items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", Some("tg-ccc00")),
        make_item("tg-ccc00", None),
    ];
    let err = reparent(&mut items, "tg-ccc00", Some("tg-aaa00".into()), &[]).unwrap_err();
    assert!(matches!(err, TgError::ParentCycle { .. }));
}

#[test]
fn reparent_accepts_valid_parent() {
    let mut items = vec![make_item("tg-aaa00", None), make_item("tg-bbb00", None)];
    reparent(&mut items, "tg-aaa00", Some("tg-bbb00".into()), &[]).unwrap();
    assert_eq!(
        items
            .iter()
            .find(|i| i.id == "tg-aaa00")
            .unwrap()
            .parent
            .as_deref(),
        Some("tg-bbb00")
    );
}

#[test]
fn reparent_clears_when_none() {
    let mut items = vec![
        make_item("tg-aaa00", Some("tg-bbb00")),
        make_item("tg-bbb00", None),
    ];
    reparent(&mut items, "tg-aaa00", None, &[]).unwrap();
    assert_eq!(
        items.iter().find(|i| i.id == "tg-aaa00").unwrap().parent,
        None
    );
}

#[test]
fn reparent_rejects_unknown_source() {
    let mut items = vec![make_item("tg-aaa00", None)];
    let err = reparent(&mut items, "tg-zzz00", None, &[]).unwrap_err();
    assert!(matches!(err, TgError::ItemNotFound(_)));
}
