use std::io::{self, IsTerminal};

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::events::{self, Event};
use task_golem::model::id;
use task_golem::model::item::Item;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(json_mode: bool, id_input: String, show_events: bool) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let items = store.load_active()?;
    let archive_ids = store.load_archive_ids()?;
    let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

    // Resolve ID with active+archive scope
    let resolved_id = id::resolve_id(&id_input, &active_ids, &archive_ids, true)?;

    // Look in active items first
    if let Some(item) = items.iter().find(|i| i.id == resolved_id).cloned() {
        if json_mode {
            output::print_json(&item);
        } else {
            output::print_item_detail(&item);
            // Append Children section (active children only; archived items
            // cannot be parents per v1 invariants).
            let children = collect_direct_children(&items, &item.id);
            if !children.is_empty() {
                output::print_children_section(&children);
            }
            if show_events {
                print_events_for(&store, &resolved_id)?;
            }
        }
        return Ok(());
    }

    // Fallback to archive
    if let Some(item) = store.load_archive_item(&resolved_id)? {
        if json_mode {
            output::print_json(&item);
        } else {
            output::print_item_detail(&item);
            // Archived items may still have active children (though the
            // invariant is that `archive` deletes aren't allowed if children
            // exist). Surface them too for symmetry.
            let children = collect_direct_children(&items, &item.id);
            if !children.is_empty() {
                output::print_children_section(&children);
            }
            if show_events {
                print_events_for(&store, &resolved_id)?;
            }
        }
        return Ok(());
    }

    Err(TgError::ItemNotFound(id_input))
}

/// Load + merge events from active and archive, sort by ts, render via the
/// shared helper. Mirrors `cli::commands::events::run` so `tg show --events`
/// and `tg events <id>` produce byte-identical event tables.
fn print_events_for(store: &Store, resolved_id: &str) -> Result<(), TgError> {
    let mut merged: Vec<Event> = Vec::new();
    merged.extend(events::read::for_task(&store.events_path(), resolved_id)?);
    merged.extend(events::read::for_task(
        &store.events_archive_path(),
        resolved_id,
    )?);
    merged.sort_by(|a, b| a.ts.cmp(&b.ts));
    let is_tty = io::stdout().is_terminal();
    output::print_events_human(&merged, is_tty);
    Ok(())
}

/// Collect direct children of `parent_id`, sorted by priority desc then id asc.
fn collect_direct_children(items: &[Item], parent_id: &str) -> Vec<Item> {
    let mut children: Vec<Item> = items
        .iter()
        .filter(|i| i.parent.as_deref() == Some(parent_id))
        .cloned()
        .collect();
    children.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    children
}
