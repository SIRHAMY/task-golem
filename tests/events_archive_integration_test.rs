//! Integration tests for the archive event-move flow:
//!
//! 1. Happy path — a task with existing events, marked done via `tg done`,
//!    has its events moved from `events.jsonl` to `events.archive.jsonl`.
//! 2. Crash-window dedup — pre-seed `events.archive.jsonl` with some events
//!    already (as if a crash occurred between the archive-append and the
//!    active-rewrite), then run the move and verify the observable state.
//! 3. Recovery sweep — a `Done` task still in the active store (e.g. due
//!    to a prior crash) gets moved by `tg archive`, and its events follow.

mod common;

use std::fs;

use common::TestProject;

fn read_jsonl(path: &std::path::Path) -> Vec<serde_json::Value> {
    if !path.exists() {
        return vec![];
    }
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid JSONL"))
        .collect()
}

/// Append a raw JSON object as one line to a JSONL file.
fn append_raw_line(path: &std::path::Path, value: &serde_json::Value) {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(f, "{}", serde_json::to_string(value).unwrap()).unwrap();
}

#[test]
fn happy_path_done_moves_events_to_archive() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Archive me"]);
    let id = added["id"].as_str().unwrap().to_string();

    // Drive a few status transitions to pile up events.
    project.run_tg_json(&["do", &id]);
    project.run_tg_json(&["block", &id, "--reason", "waiting"]);
    project.run_tg_json(&["unblock", &id]);

    let active_path = project.project_dir().join("events.jsonl");
    let archive_path = project.project_dir().join("events.archive.jsonl");

    // Sanity: events should all be in active right now.
    let before_active = read_jsonl(&active_path);
    assert!(
        before_active.iter().any(|e| e["task_id"] == id.as_str()),
        "events should exist in active before `tg done`"
    );
    assert!(
        !archive_path.exists() || read_jsonl(&archive_path).is_empty(),
        "archive events file should be empty before `tg done`"
    );

    project.run_tg_json(&["done", &id]);

    // After `tg done`: all events for this task must be in archive; none in active.
    let after_active = read_jsonl(&active_path);
    let after_archive = read_jsonl(&archive_path);

    assert!(
        !after_active.iter().any(|e| e["task_id"] == id.as_str()),
        "active should contain no events for archived task"
    );
    let archived_for_task: Vec<_> = after_archive
        .iter()
        .filter(|e| e["task_id"] == id.as_str())
        .collect();
    // Pre-existing: do, block, unblock (3) + done transition (1) = 4.
    assert_eq!(
        archived_for_task.len(),
        4,
        "expected 4 events in archive for task: {:?}",
        archived_for_task
    );
    // Chronological order preserved (sort by ts).
    let mut ts_iter = archived_for_task.iter().map(|e| e["ts"].as_str().unwrap());
    let mut prev = ts_iter.next().unwrap();
    for ts in ts_iter {
        assert!(
            prev <= ts,
            "archived events should be chronologically ascending: {} > {}",
            prev,
            ts
        );
        prev = ts;
    }
    Ok(())
}

#[test]
fn crash_window_observable_state() -> Result<(), Box<dyn std::error::Error>> {
    // Scenario: the archive-append step succeeded for some events but the
    // active-rewrite step failed (crash). The active file still has those
    // events, AND the archive file has them. When `move_for_task` runs
    // again, it will re-append (producing duplicates in archive). The
    // Phase 5 doctor check catches this; for P3 we document the observable
    // post-condition: duplicates exist across the two files, and both
    // contain the same events.
    //
    // This test validates only the observable state — not a doctor fix,
    // which lands in Phase 5.
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Crashy"]);
    let id = added["id"].as_str().unwrap().to_string();

    project.run_tg_json(&["do", &id]);

    let active_path = project.project_dir().join("events.jsonl");
    let archive_path = project.project_dir().join("events.archive.jsonl");

    // Simulate the crash window: copy the current `do` event into the
    // archive file manually (as if a prior `tg done` had appended to
    // archive but crashed before rewriting active).
    let active = read_jsonl(&active_path);
    let do_event = active
        .iter()
        .find(|e| e["task_id"] == id.as_str() && e["type"] == "status_transition")
        .expect("do event must exist")
        .clone();
    append_raw_line(&archive_path, &do_event);

    // Active still contains the event; archive now contains it too.
    let active_before = read_jsonl(&active_path);
    let archive_before = read_jsonl(&archive_path);
    assert!(active_before.iter().any(|e| e["task_id"] == id.as_str()));
    assert!(archive_before.iter().any(|e| e["task_id"] == id.as_str()));

    // Now run `tg done`, which will re-append the `do` event to archive
    // (producing a duplicate) and add the `done` event. After the move,
    // active is clean, archive has the duplicate.
    project.run_tg_json(&["done", &id]);

    let active_after = read_jsonl(&active_path);
    let archive_after = read_jsonl(&archive_path);
    assert!(
        !active_after.iter().any(|e| e["task_id"] == id.as_str()),
        "active should be clean after tg done"
    );

    // Count `do` transitions for this task in archive. Due to the crash
    // simulation we expect at least 2 (the pre-seeded copy + the re-appended
    // one from move_for_task). The duplicate-detection doctor check is the
    // Phase 5 surfacing mechanism.
    let do_count = archive_after
        .iter()
        .filter(|e| {
            e["task_id"] == id.as_str()
                && e["type"] == "status_transition"
                && e["status"] == "doing"
        })
        .count();
    assert!(
        do_count >= 2,
        "crash window should produce duplicates in archive; got {} `do` events",
        do_count
    );
    Ok(())
}

#[test]
fn recovery_sweep_moves_events_for_done_task() -> Result<(), Box<dyn std::error::Error>> {
    // Setup: create a task, do it, then simulate a stranded `Done` active
    // item (as if a prior `tg done` crashed after mutating status but
    // before archiving). We can't easily force that failure path, so we
    // hand-edit the active store to leave the item in Done status while
    // still present in tasks.jsonl. `tg archive` should then recover it
    // AND move its events.
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Stranded"]);
    let id = added["id"].as_str().unwrap().to_string();

    // Pile up a couple of events.
    project.run_tg_json(&["do", &id]);
    project.run_tg_json(&["block", &id, "--reason", "x"]);
    project.run_tg_json(&["unblock", &id]);

    // Rewrite tasks.jsonl to flip this item to Done in place.
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let tasks_contents = fs::read_to_string(&tasks_path)?;
    let mut lines: Vec<String> = tasks_contents.lines().map(|l| l.to_string()).collect();
    for line in lines.iter_mut().skip(1) {
        if line.is_empty() {
            continue;
        }
        let mut value: serde_json::Value = serde_json::from_str(line)?;
        if value["id"] == id.as_str() {
            value["status"] = serde_json::json!("done");
            value["claimed_by"] = serde_json::json!(null);
            value["claimed_at"] = serde_json::json!(null);
            *line = serde_json::to_string(&value)?;
        }
    }
    // Rewrite with trailing newline.
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    fs::write(&tasks_path, out)?;

    let active_events_path = project.project_dir().join("events.jsonl");
    let archive_events_path = project.project_dir().join("events.archive.jsonl");
    assert!(
        read_jsonl(&active_events_path)
            .iter()
            .any(|e| e["task_id"] == id.as_str()),
        "events should be in active before recovery"
    );

    let archive_result = project.run_tg_json(&["archive"]);
    assert!(archive_result["recovered"].as_u64().unwrap_or(0) >= 1);

    // Events must now be in archive, not active.
    let active_after = read_jsonl(&active_events_path);
    let archive_after = read_jsonl(&archive_events_path);
    assert!(
        !active_after.iter().any(|e| e["task_id"] == id.as_str()),
        "events should be out of active after recovery"
    );
    let archived_for_task: Vec<_> = archive_after
        .iter()
        .filter(|e| e["task_id"] == id.as_str())
        .collect();
    assert!(
        !archived_for_task.is_empty(),
        "archive should contain events for recovered task"
    );
    Ok(())
}
