//! `tg events <id> [--json]` — show the chronological event log for a task.
//!
//! Reads both `events.jsonl` (active) and `events.archive.jsonl`, merges, and
//! sorts by timestamp ascending. JSON mode emits NDJSON (one event per line)
//! preserving the on-disk byte representation; human mode renders a fixed-
//! column table via [`output::print_events_human`] with TTY-aware
//! sanitization of control bytes.

use std::io::{self, IsTerminal, Write};

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::events::{self, Event};
use task_golem::model::id;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    // Resolve ID across active ∪ archive so events for archived tasks remain
    // viewable.
    let active = store.load_active()?;
    let active_ids: Vec<String> = active.iter().map(|i| i.id.clone()).collect();
    let archive_ids = store.load_archive_ids()?;
    let resolved = id::resolve_id(&id_input, &active_ids, &archive_ids, true)?;

    let mut merged: Vec<Event> = Vec::new();
    merged.extend(events::read::for_task(&store.events_path(), &resolved)?);
    merged.extend(events::read::for_task(
        &store.events_archive_path(),
        &resolved,
    )?);
    merged.sort_by(|a, b| a.ts.cmp(&b.ts));

    if json_mode {
        print_ndjson(&merged);
    } else {
        let is_tty = io::stdout().is_terminal();
        output::print_events_human(&merged, is_tty);
    }
    Ok(())
}

/// NDJSON: one compact JSON object per line. Preserves raw text bytes
/// (no TTY sanitization) so downstream tools see exactly what's on disk.
fn print_ndjson(events: &[Event]) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    for event in events {
        // Use compact (non-pretty) form so each event is one line.
        if let Ok(line) = serde_json::to_string(event) {
            writeln!(handle, "{}", line).ok();
        }
    }
}
