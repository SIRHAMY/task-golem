//! `tg note <id> <text>` — append a free-text note event to a task.
//!
//! CLI-level invariants enforced here (the library layer is permissive):
//! - Empty text is rejected with [`TgError::InvalidInput`].
//! - Archived tasks are rejected (the note would otherwise land in
//!   `events.jsonl` for a task already in `archive.jsonl` — flagged by the
//!   Phase 5 doctor `events_in_active_for_archived_task` check).
//!
//! The library-level [`Store::append_note`] already enforces "task must
//! exist in active" by checking `load_active`; we surface the archive-only
//! case here with a more user-friendly error message before delegating.

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::events::Event;
use task_golem::model::id;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(json_mode: bool, id_input: String, text: String) -> Result<(), TgError> {
    if text.is_empty() {
        return Err(TgError::InvalidInput(
            "note text cannot be empty".to_string(),
        ));
    }

    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    // Resolve ID against active+archive so we can give a precise error for
    // archive-only matches before append_note rejects with the more generic
    // ItemNotFound. The store lock is not required for the read paths here;
    // append_note itself relies on O_APPEND atomicity.
    let active = store.load_active()?;
    let active_ids: Vec<String> = active.iter().map(|i| i.id.clone()).collect();
    let archive_ids = store.load_archive_ids()?;
    let resolved = id::resolve_id(&id_input, &active_ids, &archive_ids, true)?;

    if !active_ids.iter().any(|id| id == &resolved) {
        // ID resolved but only to archive — reject per DESIGN's archived-task
        // note rule.
        return Err(TgError::InvalidInput(format!(
            "Cannot add note to archived task {}: notes are only permitted on active tasks",
            resolved
        )));
    }

    let event = store.append_note(&resolved, &text)?;
    print_success(json_mode, &event);
    Ok(())
}

fn print_success(json_mode: bool, event: &Event) {
    if json_mode {
        output::print_json(event);
    } else {
        output::print_human(&format!("Noted: {} - {}", event.task_id, event.text));
    }
}
