use chrono::{DateTime, NaiveDate, Utc};

use crate::cli::output;
use crate::errors::TgError;
use crate::model::item::Item;
use crate::model::status::Status;
use crate::store::root;
use crate::store::Store;

/// Handle `tg archive [--before DATE]`
///
/// 1. Scan active store for done items not yet archived (edge case recovery).
/// 2. If --before DATE, prune archive entries older than DATE to archive-pruned.jsonl.
pub fn run(json_mode: bool, before: Option<String>) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir.clone());

    // Parse --before date if provided
    let before_date: Option<DateTime<Utc>> = if let Some(ref date_str) = before {
        let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|e| {
            TgError::InvalidInput(format!(
                "Invalid date '{}': expected ISO 8601 format (YYYY-MM-DD): {}",
                date_str, e
            ))
        })?;
        Some(
            naive
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc(),
        )
    } else {
        None
    };

    let result = store.with_lock(|store| {
        // Step 1: Recover unarchived done items from active store
        let mut active_items = store.load_active()?;
        let mut recovered: Vec<Item> = Vec::new();

        let mut i = 0;
        while i < active_items.len() {
            if active_items[i].status == Status::Done {
                let done_item = active_items.remove(i);
                store.append_to_archive(&done_item)?;
                recovered.push(done_item);
            } else {
                i += 1;
            }
        }

        if !recovered.is_empty() {
            store.save_active(&active_items)?;
        }

        // Step 2: Prune archive if --before provided
        let mut pruned: Vec<Item> = Vec::new();
        if let Some(cutoff) = before_date {
            let all_archive = store.load_all_archive()?;
            let mut keep: Vec<Item> = Vec::new();

            for item in all_archive {
                if item.updated_at < cutoff {
                    pruned.push(item);
                } else {
                    keep.push(item);
                }
            }

            if !pruned.is_empty() {
                // Write pruned items to archive-pruned.jsonl
                let pruned_path = project_dir.join("archive-pruned.jsonl");
                // Append to existing pruned file if it exists
                for pruned_item in &pruned {
                    crate::store::jsonl::append_to_archive(&pruned_path, pruned_item)?;
                }

                // Rewrite archive without pruned items
                crate::store::jsonl::write_atomic(&store.archive_path(), &keep)?;
            }
        }

        Ok(ArchiveResult {
            recovered_count: recovered.len(),
            recovered_ids: recovered.iter().map(|i| i.id.clone()).collect(),
            pruned_count: pruned.len(),
            pruned_ids: pruned.iter().map(|i| i.id.clone()).collect(),
        })
    })?;

    if json_mode {
        output::print_json(&serde_json::json!({
            "recovered": result.recovered_count,
            "recovered_ids": result.recovered_ids,
            "pruned": result.pruned_count,
            "pruned_ids": result.pruned_ids,
        }));
    } else {
        if result.recovered_count > 0 {
            output::print_human(&format!(
                "Recovered {} done item(s) to archive: {}",
                result.recovered_count,
                result.recovered_ids.join(", ")
            ));
        }
        if result.pruned_count > 0 {
            output::print_human(&format!(
                "Pruned {} item(s) to archive-pruned.jsonl: {}",
                result.pruned_count,
                result.pruned_ids.join(", ")
            ));
        }
        if result.recovered_count == 0 && result.pruned_count == 0 {
            output::print_human("No items to archive or prune.");
        }
    }

    Ok(())
}

struct ArchiveResult {
    recovered_count: usize,
    recovered_ids: Vec<String>,
    pruned_count: usize,
    pruned_ids: Vec<String>,
}
