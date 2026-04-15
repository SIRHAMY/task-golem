//! Move events from `events.jsonl` to `events.archive.jsonl` when their task
//! is archived.
//!
//! # Contract
//!
//! [`move_for_task`] reads the active events file, partitions events into
//! *keep* and *move* by `task_id`, appends moved events to the archive file
//! (via [`crate::events::append::write`], preserving the single-`write(2)`
//! plus-`O_APPEND` contract), and then atomically rewrites the active file
//! with the kept events.
//!
//! # Recovery / crash windows
//!
//! The two-step "append to archive, then rewrite active" is not a single
//! atomic step. A crash after the append but before the rewrite leaves
//! duplicate events in both files. This is deliberately acknowledged and
//! surfaced by the Phase 5 `events_dup_across_active_and_archive` doctor
//! check; callers that care (e.g., the store chokepoint) must still call
//! this under the same [`crate::store::Store::with_lock`] closure that
//! guards the task mutation.
//!
//! # Lenient read
//!
//! [`crate::events::read::all`] is lenient — malformed lines warn-and-skip.
//! A corrupt active events file therefore cannot prevent the move from
//! making progress on valid lines, but it *will* drop the corrupt lines
//! during the rewrite (they are not partitioned into *keep*). This is the
//! correct behavior: the doctor check surfaces the drop, and the alternative
//! (refusing the move on any corruption) would strand completed tasks.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::errors::TgError;
use crate::events::record::Event;
use crate::events::{append as events_append, read as events_read};

/// Move every event belonging to `task_id` from `active_path` to
/// `archive_path`. Returns the number of events moved.
///
/// Steps:
///
/// 1. Read all events from `active_path` (lenient — malformed lines are
///    skipped).
/// 2. Partition into *keep* (all events where `task_id` does not match) and
///    *move*.
/// 3. If *move* is empty, return `Ok(0)` — no I/O, no rewrite.
/// 4. For each event in *move*, append to `archive_path` via
///    [`events_append::write`] (single `write(2)` + `sync_data`).
/// 5. Atomically rewrite `active_path` with *keep* (temp file in the same
///    directory, `sync_all`, rename).
///
/// The rewrite uses a local headerless atomic-write helper rather than
/// generalizing `jsonl::write_atomic` — the events file format has no
/// schema header, and we want to keep the store's JSONL helpers unaware of
/// events.
///
/// # Locking
///
/// Must be called from inside the caller's `with_lock` closure. The
/// function does not acquire its own lock.
pub fn move_for_task(
    active_path: &Path,
    archive_path: &Path,
    task_id: &str,
) -> Result<usize, TgError> {
    // Step 1-2: read + partition. `events::read::all` returns events sorted
    // by `ts` ascending; we preserve that order when appending to archive,
    // which keeps the archive roughly chronological per task.
    let all = events_read::all(active_path)?;

    let (moved, keep): (Vec<Event>, Vec<Event>) =
        all.into_iter().partition(|e| e.task_id == task_id);

    if moved.is_empty() {
        return Ok(0);
    }

    // Step 4: append each moved event to archive with the single-write
    // contract. If this fails partway through, some events will be in
    // archive and some still in active — the duplicate-detection doctor
    // check covers this window.
    for event in &moved {
        events_append::write(archive_path, event)?;
    }

    // Step 5: atomically rewrite active with kept events.
    rewrite_events_file_atomic(active_path, &keep)?;

    Ok(moved.len())
}

/// Atomically rewrite an events file with `events`. Headerless (events have
/// no schema header on the file itself; every line carries `v`).
///
/// Writes each event as a single JSON line terminated by `\n`, then
/// `sync_all`s the temp file and atomically renames over `path`. If
/// `events` is empty, writes an empty file (this is the steady state for
/// a just-archived task with no remaining events).
fn rewrite_events_file_atomic(path: &Path, events: &[Event]) -> Result<(), TgError> {
    let dir = path.parent().ok_or_else(|| {
        TgError::IoError(std::io::Error::other(
            "Cannot determine parent directory for atomic events rewrite",
        ))
    })?;

    // Ensure the directory exists; the events file may be rewritten before
    // any prior write if a move is the first operation after `tg init`.
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(TgError::IoError)?;
    }

    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(TgError::IoError)?;

    for event in events {
        let line = serde_json::to_string(event)
            .expect("Event serialization cannot fail: all fields are safely typed");
        // BufWriter/write_all is fine here — we're writing our own temp
        // file under no concurrent append pressure (the caller holds the
        // lock). The single-write contract applies only to the append path.
        tmp.write_all(line.as_bytes()).map_err(TgError::IoError)?;
        tmp.write_all(b"\n").map_err(TgError::IoError)?;
    }

    // fsync before rename — durability guarantee mirrors `jsonl::write_atomic`.
    tmp.as_file().sync_all().map_err(TgError::IoError)?;

    tmp.persist(path).map_err(|e| TgError::IoError(e.error))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::events::record::Event;
    use crate::model::status::Status;

    #[test]
    fn move_noop_when_no_events_match() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");
        events_append::write(&active, &Event::note("tg-aaa00", "alice", "a")).unwrap();

        let moved = move_for_task(&active, &archive, "tg-zzz00").unwrap();
        assert_eq!(moved, 0);
        // Active untouched (still has the one event), archive not created.
        let remaining = events_read::all(&active).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(!archive.exists());
    }

    #[test]
    fn move_noop_when_active_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");
        assert!(!active.exists());

        let moved = move_for_task(&active, &archive, "tg-aaa00").unwrap();
        assert_eq!(moved, 0);
        assert!(!archive.exists());
    }

    #[test]
    fn move_partitions_by_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");

        events_append::write(&active, &Event::note("tg-aaa00", "alice", "a1")).unwrap();
        events_append::write(&active, &Event::note("tg-bbb00", "alice", "b1")).unwrap();
        events_append::write(
            &active,
            &Event::status_transition("tg-aaa00", "alice", Status::Done, ""),
        )
        .unwrap();
        events_append::write(&active, &Event::note("tg-bbb00", "alice", "b2")).unwrap();

        let moved = move_for_task(&active, &archive, "tg-aaa00").unwrap();
        assert_eq!(moved, 2);

        let remaining = events_read::all(&active).unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().all(|e| e.task_id == "tg-bbb00"));

        let archived = events_read::all(&archive).unwrap();
        assert_eq!(archived.len(), 2);
        assert!(archived.iter().all(|e| e.task_id == "tg-aaa00"));
    }

    #[test]
    fn move_preserves_other_tasks_events() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");

        events_append::write(&active, &Event::note("tg-keep0", "alice", "keep")).unwrap();
        events_append::write(&active, &Event::note("tg-move0", "alice", "move")).unwrap();

        move_for_task(&active, &archive, "tg-move0").unwrap();

        let remaining = events_read::all(&active).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].text, "keep");
    }

    #[test]
    fn repeated_move_is_idempotent_on_second_call() {
        // After the first move, the active file has no more events for this
        // task; the second move is a no-op and doesn't re-append duplicates
        // to archive.
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");

        events_append::write(&active, &Event::note("tg-abc00", "alice", "one")).unwrap();
        assert_eq!(move_for_task(&active, &archive, "tg-abc00").unwrap(), 1);
        assert_eq!(move_for_task(&active, &archive, "tg-abc00").unwrap(), 0);

        let archived = events_read::all(&archive).unwrap();
        assert_eq!(archived.len(), 1);
    }

    #[test]
    fn move_survives_malformed_line_in_active() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");

        events_append::write(&active, &Event::note("tg-abc00", "alice", "good")).unwrap();
        // Inject a malformed trailing line to simulate a crash mid-append.
        {
            let mut f = fs::OpenOptions::new().append(true).open(&active).unwrap();
            f.write_all(b"{not json\n").unwrap();
        }

        let moved = move_for_task(&active, &archive, "tg-abc00").unwrap();
        assert_eq!(moved, 1);

        // After the rewrite, the malformed line is dropped (the lenient
        // reader skipped it, so it wasn't partitioned into `keep` either).
        let remaining = fs::read_to_string(&active).unwrap();
        assert!(!remaining.contains("not json"));
    }

    #[test]
    fn rewrite_produces_empty_file_when_all_events_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let active = tmp.path().join("events.jsonl");
        let archive = tmp.path().join("events.archive.jsonl");

        events_append::write(&active, &Event::note("tg-abc00", "alice", "a")).unwrap();
        events_append::write(&active, &Event::note("tg-abc00", "alice", "b")).unwrap();

        move_for_task(&active, &archive, "tg-abc00").unwrap();

        let remaining = events_read::all(&active).unwrap();
        assert!(remaining.is_empty());
        let contents = fs::read_to_string(&active).unwrap();
        assert!(contents.is_empty() || contents.trim().is_empty());
    }
}
