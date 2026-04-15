//! Chokepoint regression test: every public status-mutating verb emits
//! exactly one `status_transition` event via `Store::commit_*`.
//!
//! This guards against future refactors that bypass the witness (e.g.
//! calling `save_active` directly after a status mutation). The test is
//! parameterized over the five verbs and includes a meta-assertion that
//! counts `fn apply_` occurrences in `src/model/item.rs` — if a sixth
//! `apply_*` is added to `Item` without updating the verb array below, the
//! meta-assertion fails and forces the author to decide how the new verb
//! interacts with events.

mod common;

use std::fs;

use common::TestProject;

/// Read `events.jsonl` as a list of JSON objects. Missing file → empty vec.
fn read_events(project_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let path = project_dir.join("events.jsonl");
    if !path.exists() {
        return vec![];
    }
    let contents = fs::read_to_string(&path).expect("read events.jsonl");
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("every event line must be valid JSON"))
        .collect()
}

/// Read `events.archive.jsonl` (for verbs that archive — `done`).
fn read_archive_events(project_dir: &std::path::Path) -> Vec<serde_json::Value> {
    let path = project_dir.join("events.archive.jsonl");
    if !path.exists() {
        return vec![];
    }
    let contents = fs::read_to_string(&path).expect("read events.archive.jsonl");
    contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("every archive event line must be valid JSON"))
        .collect()
}

#[test]
fn verb_do_emits_status_transition_event() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Item"]);
    let id = added["id"].as_str().unwrap().to_string();

    // Baseline: no events before the transition.
    assert!(read_events(&project.project_dir()).is_empty());

    project.run_tg_json(&["do", &id]);

    let events = read_events(&project.project_dir());
    assert_eq!(events.len(), 1, "expected exactly one event for `tg do`");
    assert_eq!(events[0]["type"], "status_transition");
    assert_eq!(events[0]["task_id"], id);
    assert_eq!(events[0]["status"], "doing");
    Ok(())
}

#[test]
fn verb_done_emits_status_transition_event_in_archive() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Item"]);
    let id = added["id"].as_str().unwrap().to_string();

    project.run_tg_json(&["done", &id]);

    // The event is first written to events.jsonl, then moved to
    // events.archive.jsonl by `events::archive::move_for_task`. After
    // `tg done` completes, the active file should have no events for this
    // task and the archive file should have exactly one.
    let active = read_events(&project.project_dir());
    let archive = read_archive_events(&project.project_dir());

    let for_this_task_active: Vec<_> = active
        .iter()
        .filter(|e| e["task_id"] == id.as_str())
        .collect();
    let for_this_task_archive: Vec<_> = archive
        .iter()
        .filter(|e| e["task_id"] == id.as_str())
        .collect();

    assert!(
        for_this_task_active.is_empty(),
        "active events should be empty for archived task: {:?}",
        for_this_task_active
    );
    assert_eq!(
        for_this_task_archive.len(),
        1,
        "expected exactly one archived event for `tg done`"
    );
    assert_eq!(for_this_task_archive[0]["type"], "status_transition");
    assert_eq!(for_this_task_archive[0]["status"], "done");
    Ok(())
}

#[test]
fn verb_todo_emits_status_transition_event() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Item"]);
    let id = added["id"].as_str().unwrap().to_string();

    // Transition to doing first so `tg todo` has a non-idempotent target.
    project.run_tg_json(&["do", &id]);
    let baseline = read_events(&project.project_dir()).len();

    project.run_tg_json(&["todo", &id]);

    let events = read_events(&project.project_dir());
    assert_eq!(
        events.len() - baseline,
        1,
        "expected exactly one new event for `tg todo`"
    );
    let latest = events.last().unwrap();
    assert_eq!(latest["type"], "status_transition");
    assert_eq!(latest["task_id"], id);
    assert_eq!(latest["status"], "todo");
    Ok(())
}

#[test]
fn verb_block_emits_status_transition_event() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Item"]);
    let id = added["id"].as_str().unwrap().to_string();

    project.run_tg_json(&["block", &id, "--reason", "waiting"]);

    let events = read_events(&project.project_dir());
    assert_eq!(events.len(), 1, "expected exactly one event for `tg block`");
    assert_eq!(events[0]["type"], "status_transition");
    assert_eq!(events[0]["task_id"], id);
    assert_eq!(events[0]["status"], "blocked");
    // The witness carries the blocking reason as the event text.
    assert_eq!(events[0]["text"], "waiting");
    Ok(())
}

#[test]
fn verb_unblock_emits_status_transition_event() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Item"]);
    let id = added["id"].as_str().unwrap().to_string();

    project.run_tg_json(&["block", &id, "--reason", "waiting"]);
    let baseline = read_events(&project.project_dir()).len();

    project.run_tg_json(&["unblock", &id]);

    let events = read_events(&project.project_dir());
    assert_eq!(
        events.len() - baseline,
        1,
        "expected exactly one new event for `tg unblock`"
    );
    let latest = events.last().unwrap();
    assert_eq!(latest["type"], "status_transition");
    assert_eq!(latest["task_id"], id);
    // Unblock restores prior state — item was in `todo` before block,
    // so it restores to `todo`.
    assert_eq!(latest["status"], "todo");
    Ok(())
}

/// Meta-assertion: the verb array this test implicitly defines must match
/// the number of `apply_*` methods in `src/model/item.rs`. If a sixth
/// `apply_*` is added without updating this test, fail loudly so the
/// author explicitly decides how the new verb interacts with events.
#[test]
fn apply_method_count_matches_covered_verbs() {
    // Verbs covered above: do, done, todo, block, unblock → 5.
    const COVERED_VERBS: usize = 5;

    // `CARGO_MANIFEST_DIR` is the crate root (tests/ sibling to src/).
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let item_src = std::path::Path::new(manifest_dir).join("src/model/item.rs");
    let contents = fs::read_to_string(&item_src).expect("read src/model/item.rs");

    // Count occurrences of `    pub fn apply_` — the leading indent avoids
    // matching random mentions of `apply_` in comments/docstrings.
    let count = contents.matches("    pub fn apply_").count();

    assert_eq!(
        count, COVERED_VERBS,
        "Item has {} apply_* methods; chokepoint test covers {}. Update \
         events_chokepoint_test.rs to include the new verb and assert its \
         status_transition event shape.",
        count, COVERED_VERBS
    );
}
