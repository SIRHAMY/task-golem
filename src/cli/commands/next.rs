use crate::cli::output;
use crate::errors::TgError;
use crate::model::deps;
use crate::store::root;
use crate::store::Store;

pub fn run(json_mode: bool, verbose: bool) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    if verbose {
        eprintln!("[verbose] Project root: {}", project_dir.display());
    }
    let store = Store::new(project_dir);

    // Read-only operation: no lock needed
    let active_items = store.load_active()?;
    let archive_ids = store.load_archive_ids()?;
    if verbose {
        eprintln!(
            "[verbose] Loaded {} active items, {} archive IDs",
            active_items.len(),
            archive_ids.len()
        );
    }

    let (ready, warnings) = deps::compute_ready_queue(&active_items, &archive_ids);

    for w in &warnings {
        eprintln!("{}", w);
    }

    let next_item = ready.into_iter().next();

    if json_mode {
        output::print_json(&next_item);
    } else if let Some(item) = &next_item {
        output::print_human(&format!(
            "{} [{}] (p:{}) {}",
            item.id, item.status, item.priority, item.title
        ));
    } else {
        output::print_human("No items ready");
    }

    Ok(())
}
