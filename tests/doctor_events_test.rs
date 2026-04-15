//! Doctor integrity checks for TG-008 events (Phase 5).
//!
//! Seeds each of the five failure modes directly into `events.jsonl` /
//! `events.archive.jsonl` via `TestProject::seed_raw_events` and asserts the
//! corresponding doctor check fires. Also verifies `--fix` repairs the two
//! conditions that have automatic repairs.

mod common;

use std::fs;

use chrono::{Duration, Utc};
use common::TestProject;
use task_golem::events::Event;
use task_golem::model::status::Status;

fn issues_of_type<'a>(report: &'a serde_json::Value, ty: &str) -> Vec<&'a serde_json::Value> {
    report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["type"] == ty)
        .collect()
}

#[test]
fn doctor_detects_events_malformed() {
    let project = TestProject::new().unwrap();
    project.run_tg_json(&["add", "Task A"]);

    // Write a malformed middle line directly.
    let events_path = project.project_dir().join("events.jsonl");
    let good = r#"{"v":1,"task_id":"tg-aaa00","ts":"2026-04-15T12:00:00.000000Z","author":"t","type":"note","text":"ok"}"#;
    fs::write(&events_path, format!("{good}\n{{not json\n")).unwrap();

    let report = project.run_tg_json(&["doctor"]);
    let malformed = issues_of_type(&report, "events_malformed");
    assert!(
        !malformed.is_empty(),
        "expected events_malformed issue, got: {:?}",
        report["issues"]
    );
}

#[test]
fn doctor_detects_events_drift_status_mismatch() {
    let project = TestProject::new().unwrap();
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap().to_string();

    // Task is Todo. Seed a status_transition event claiming it went to Done.
    let mismatched = Event::status_transition(id.clone(), "tester", Status::Done, "");
    project.seed_raw_events(&[mismatched], &[]);

    let report = project.run_tg_json(&["doctor"]);
    let drift = issues_of_type(&report, "events_drift_status_mismatch");
    assert!(
        !drift.is_empty(),
        "expected events_drift_status_mismatch issue, got: {:?}",
        report["issues"]
    );
}

#[test]
fn doctor_detects_events_orphan() {
    let project = TestProject::new().unwrap();
    project.run_tg_json(&["add", "Task A"]);

    // Seed an event for a task_id that doesn't exist in active/archive/pruned.
    let orphan = Event::note("tg-zzz99", "tester", "orphan");
    project.seed_raw_events(&[orphan], &[]);

    let report = project.run_tg_json(&["doctor"]);
    let orphans = issues_of_type(&report, "events_orphan");
    assert!(
        !orphans.is_empty(),
        "expected events_orphan issue, got: {:?}",
        report["issues"]
    );
}

#[test]
fn doctor_orphan_check_respects_archive_pruned() {
    let project = TestProject::new().unwrap();

    // Create an archive-pruned.jsonl with one task. Events for that task must
    // NOT surface as orphans.
    let pruned_path = project.project_dir().join("archive-pruned.jsonl");
    let pruned_line = r#"{"id":"tg-pru00","title":"pruned","status":"done","priority":0,"description":null,"tags":[],"dependencies":[],"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","blocked_reason":null,"blocked_from_status":null,"claimed_by":null,"claimed_at":null}"#;
    fs::write(&pruned_path, format!("{}\n", pruned_line)).unwrap();

    let pruned_event = Event::note("tg-pru00", "tester", "from before pruning");
    project.seed_raw_events(&[], &[pruned_event]);

    let report = project.run_tg_json(&["doctor"]);
    let orphans = issues_of_type(&report, "events_orphan");
    assert!(
        orphans.is_empty(),
        "pruned task events should not be orphans, got: {:?}",
        orphans
    );
}

#[test]
fn doctor_detects_events_in_active_for_archived_task_and_fix_moves_them() {
    let project = TestProject::new().unwrap();

    // Create + done + archive a task so it's in archive.jsonl.
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap().to_string();
    project.run_tg(&["do", &id]);
    project.run_tg(&["done", &id]);
    // After `tg done`, the task is already archived AND its events moved.
    // Seed a late event in active to simulate an append that slipped through
    // the archive-move crash window.
    let late = Event::note(id.clone(), "tester", "late");
    // Preserve anything that already exists in archive events by reading it
    // first.
    let archive_path = project.project_dir().join("events.archive.jsonl");
    let archive_before: Vec<Event> = if archive_path.exists() {
        std::fs::read_to_string(&archive_path)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    } else {
        vec![]
    };
    project.seed_raw_events(&[late.clone()], &archive_before);

    let report = project.run_tg_json(&["doctor"]);
    let stranded = issues_of_type(&report, "events_in_active_for_archived_task");
    assert!(
        !stranded.is_empty(),
        "expected events_in_active_for_archived_task, got: {:?}",
        report["issues"]
    );

    // --fix moves them to archive.
    let fix_output = project.run_tg(&["--json", "doctor", "--fix"]);
    assert!(
        fix_output.status.success(),
        "doctor --fix should succeed: {}",
        String::from_utf8_lossy(&fix_output.stderr)
    );

    let post = project.run_tg_json(&["doctor"]);
    let remaining = issues_of_type(&post, "events_in_active_for_archived_task");
    assert!(
        remaining.is_empty(),
        "expected stranded events repaired, got: {:?}",
        post["issues"]
    );

    // Assert `tg events <id>` still shows the moved event (sanity).
    let events_out = project.run_tg(&["events", &id, "--json"]);
    let stdout = String::from_utf8_lossy(&events_out.stdout);
    assert!(
        stdout.contains("late"),
        "moved event text should still be readable via tg events: {}",
        stdout
    );
}

#[test]
fn doctor_detects_events_dup_across_files_and_fix_drops_from_active() {
    let project = TestProject::new().unwrap();
    project.run_tg_json(&["add", "Task A"]);

    // Construct the same event (same task_id, ts, text) in both files.
    let ts = Utc::now() - Duration::minutes(1);
    let mut shared = Event::note("tg-dup99", "tester", "dup");
    shared.ts = ts;

    project.seed_raw_events(&[shared.clone()], &[shared.clone()]);

    let report = project.run_tg_json(&["doctor"]);
    let dups = issues_of_type(&report, "events_dup_across_active_and_archive");
    assert!(
        !dups.is_empty(),
        "expected events_dup_across_active_and_archive, got: {:?}",
        report["issues"]
    );

    let fix_output = project.run_tg(&["--json", "doctor", "--fix"]);
    assert!(
        fix_output.status.success(),
        "doctor --fix should succeed: {}",
        String::from_utf8_lossy(&fix_output.stderr)
    );

    // After --fix the dup is removed from active; archive retains it.
    let active_path = project.project_dir().join("events.jsonl");
    let active_content = fs::read_to_string(&active_path).unwrap_or_default();
    assert!(
        !active_content.contains("\"tg-dup99\""),
        "dup should be removed from active, got: {}",
        active_content
    );

    let archive_path = project.project_dir().join("events.archive.jsonl");
    let archive_content = fs::read_to_string(&archive_path).unwrap_or_default();
    assert!(
        archive_content.contains("\"tg-dup99\""),
        "dup should remain in archive, got: {}",
        archive_content
    );

    let post = project.run_tg_json(&["doctor"]);
    let remaining = issues_of_type(&post, "events_dup_across_active_and_archive");
    assert!(
        remaining.is_empty(),
        "expected dup repaired, got: {:?}",
        post["issues"]
    );
}

#[test]
fn doctor_fix_does_not_touch_drift_or_orphan() {
    // Repairs are only automatic for events_in_active_for_archived_task and
    // events_dup_across_active_and_archive. drift / orphan / malformed must
    // persist after --fix.
    let project = TestProject::new().unwrap();
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap().to_string();

    let drift = Event::status_transition(id.clone(), "tester", Status::Done, "");
    let orphan = Event::note("tg-zzz99", "tester", "orphan");
    project.seed_raw_events(&[drift, orphan], &[]);

    let fix_output = project.run_tg(&["--json", "doctor", "--fix"]);
    assert!(fix_output.status.success());

    let post = project.run_tg_json(&["doctor"]);
    let drift_after = issues_of_type(&post, "events_drift_status_mismatch");
    let orphan_after = issues_of_type(&post, "events_orphan");
    assert!(
        !drift_after.is_empty(),
        "drift should persist after --fix, got: {:?}",
        post["issues"]
    );
    assert!(
        !orphan_after.is_empty(),
        "orphan should persist after --fix, got: {:?}",
        post["issues"]
    );
}
