use chrono::Utc;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::deps;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(
    json_mode: bool,
    verbose: bool,
    include_stale: Option<String>,
    limit: Option<usize>,
) -> Result<(), TgError> {
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

    let (mut ready, warnings) = deps::compute_ready_queue(&active_items, &archive_ids);

    // Emit warnings from ready queue computation
    for w in &warnings {
        eprintln!("{}", w);
    }

    // Include stale doing items if requested
    if let Some(ref stale_str) = include_stale {
        let duration: std::time::Duration = stale_str
            .parse::<humantime::Duration>()
            .map_err(|e| TgError::InvalidInput(format!("Invalid duration '{}': {}", stale_str, e)))?
            .into();
        let threshold = Utc::now()
            - chrono::Duration::from_std(duration)
                .map_err(|e| TgError::InvalidInput(format!("Duration too large: {}", e)))?;

        let mut stale_doing: Vec<Item> = active_items
            .iter()
            .filter(|item| item.status == Status::Doing && item.updated_at < threshold)
            .cloned()
            .collect();

        // Sort stale items the same way
        stale_doing.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        if verbose {
            eprintln!(
                "[verbose] Including {} stale doing items",
                stale_doing.len()
            );
        }

        ready.extend(stale_doing);
    }

    // Apply limit
    if let Some(n) = limit {
        ready.truncate(n);
    }

    if verbose {
        eprintln!("[verbose] Ready queue: {} items", ready.len());
    }

    if json_mode {
        output::print_json(&ready);
    } else if ready.is_empty() {
        output::print_human("No items ready");
    } else {
        output::print_item_table(&ready);
    }

    Ok(())
}
