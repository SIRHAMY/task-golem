use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::id;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(
    json_mode: bool,
    verbose: bool,
    status_filter: Option<String>,
    tag_filter: Option<String>,
    parent_filter: Option<String>,
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

    // Resolve parent ID up-front against the full active+archive ID space so
    // prefix/bare-hex resolution doesn't fail when `--status` narrows the
    // item set below the parent itself.
    let resolved_parent = if let Some(ref parent_input) = parent_filter {
        let active = store.load_active()?;
        let active_ids: Vec<String> = active.iter().map(|i| i.id.clone()).collect();
        let archive_ids = store.load_archive_ids()?;
        Some(id::resolve_id(
            parent_input,
            &active_ids,
            &archive_ids,
            true,
        )?)
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

    // Apply parent filter (direct children only).
    if let Some(ref parent_id) = resolved_parent {
        items.retain(|item| item.parent.as_deref() == Some(parent_id.as_str()));
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
