use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::status::Status;
use task_golem::store::root;
use task_golem::store::Store;

pub fn run(
    json_mode: bool,
    verbose: bool,
    status_filter: Option<String>,
    tag_filter: Option<String>,
) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    if verbose {
        eprintln!("[verbose] Project root: {}", project_dir.display());
    }
    let store = Store::new(project_dir);

    // Parse status filter if provided
    let parsed_status = if let Some(ref s) = status_filter {
        let status: Status = s.parse().map_err(TgError::InvalidInput)?;
        Some(status)
    } else {
        None
    };

    // Load items based on status filter
    let mut items = if parsed_status == Some(Status::Done) {
        // Load full archive for done items
        let archive = store.load_all_archive()?;
        if verbose {
            eprintln!("[verbose] Loaded {} items from archive", archive.len());
        }
        archive
    } else {
        let active = store.load_active()?;
        if verbose {
            eprintln!("[verbose] Loaded {} active items", active.len());
        }
        active
    };

    // Apply status filter
    if let Some(ref status) = parsed_status {
        items.retain(|item| item.status == *status);
    } else {
        // Default: all non-done active items
        items.retain(|item| item.status != Status::Done);
    }

    // Apply tag filter
    if let Some(ref tag) = tag_filter {
        items.retain(|item| item.tags.contains(tag));
    }

    // Sort: priority desc, then created_at asc
    items.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });

    if verbose {
        eprintln!("[verbose] Returning {} items after filtering", items.len());
    }

    if json_mode {
        output::print_json(&items);
    } else if items.is_empty() {
        output::print_human("No items found.");
    } else {
        output::print_item_table(&items);
    }

    Ok(())
}
