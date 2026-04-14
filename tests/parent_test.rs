//! Parent field tests: Phase 1 covers serde + reparent orchestration (data-model
//! layer). Phase 2 adds CLI-level tests that exercise `tg add`/`edit`/`rm`/`show`
//! via the `TestProject` harness.

mod common;

use common::TestProject;

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

// ---------- Phase 2: CLI-level tests ----------

#[test]
fn cli_add_with_parent_sets_parent() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Parent"]);
    let pid = p["id"].as_str().unwrap().to_string();

    let c = proj.run_tg_json(&["add", "Child", "--parent", &pid]);
    assert_eq!(c["parent"].as_str().unwrap(), pid);

    let shown = proj.run_tg_json(&["show", c["id"].as_str().unwrap()]);
    assert_eq!(shown["parent"].as_str().unwrap(), pid);
}

#[test]
fn cli_add_with_bogus_parent_errors() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["add", "Orphan", "--parent", "tg-zzz99"]);
    assert!(!output.status.success());
    // Should be AmbiguousId/ItemNotFound from resolve_id or ParentDangling.
    // Either way, a nonzero exit and the tasks.jsonl should not have the new
    // item persisted.
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn cli_edit_parent_sets_and_clears() {
    let proj = TestProject::new().unwrap();
    let a = proj.run_tg_json(&["add", "A"]);
    let aid = a["id"].as_str().unwrap().to_string();
    let b = proj.run_tg_json(&["add", "B"]);
    let bid = b["id"].as_str().unwrap().to_string();

    // Set parent
    let edited = proj.run_tg_json(&["edit", &aid, "--parent", &bid]);
    assert_eq!(edited["parent"].as_str().unwrap(), bid);

    // Clear parent
    let edited = proj.run_tg_json(&["edit", &aid, "--parent-clear"]);
    assert!(edited["parent"].is_null());
}

#[test]
fn cli_edit_parent_and_clear_mutually_exclusive() {
    let proj = TestProject::new().unwrap();
    let a = proj.run_tg_json(&["add", "A"]);
    let aid = a["id"].as_str().unwrap().to_string();
    let b = proj.run_tg_json(&["add", "B"]);
    let bid = b["id"].as_str().unwrap().to_string();

    let output = proj.run_tg(&["edit", &aid, "--parent", &bid, "--parent-clear"]);
    assert!(!output.status.success());
}

#[test]
fn cli_edit_rejects_cycle() {
    let proj = TestProject::new().unwrap();
    let a = proj.run_tg_json(&["add", "A"]);
    let aid = a["id"].as_str().unwrap().to_string();
    let b = proj.run_tg_json(&["add", "B", "--parent", &aid]);
    let bid = b["id"].as_str().unwrap().to_string();

    // Setting A.parent = B would create a cycle.
    let output = proj.run_tg(&["edit", &aid, "--parent", &bid]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_lowercase().contains("cycle"));
}

#[test]
fn cli_rm_rejected_when_children_exist() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Parent"]);
    let pid = p["id"].as_str().unwrap().to_string();
    let c = proj.run_tg_json(&["add", "Child", "--parent", &pid]);
    let cid = c["id"].as_str().unwrap().to_string();

    let output = proj.run_tg(&["rm", &pid]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&cid));

    // Remove child first; then parent removes cleanly.
    proj.run_tg_json(&["rm", &cid]);
    let rm_json = proj.run_tg_json(&["rm", &pid]);
    assert_eq!(rm_json["removed"].as_str().unwrap(), pid);
}

#[test]
fn cli_show_children_section_rendered() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Epic"]);
    let pid = p["id"].as_str().unwrap().to_string();
    proj.run_tg_json(&["add", "Child A", "--parent", &pid]);
    proj.run_tg_json(&["add", "Child B", "--parent", &pid]);

    let output = proj.run_tg(&["show", &pid]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Children:"));
    assert!(stdout.contains("Child A"));
    assert!(stdout.contains("Child B"));
}

#[test]
fn cli_show_leaf_omits_children_section() {
    let proj = TestProject::new().unwrap();
    let a = proj.run_tg_json(&["add", "Solo"]);
    let aid = a["id"].as_str().unwrap();

    let output = proj.run_tg(&["show", aid]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Children:"));
}
