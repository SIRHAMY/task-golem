use chrono::{DateTime, NaiveDate, Utc};

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::store::root;

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
        Some(naive.and_hms_opt(0, 0, 0).unwrap().and_utc())
    } else {
        None
    };

    let result = store.with_lock(|store| {
        // Step 1: Recover unarchived done items from active store.
        //
        // A done candidate is SKIPPED if any non-done active item still has it
        // as a parent. This prevents the sweep from orphaning active children
        // during cleanup. Skipped candidates emit a warning to stderr; the sweep
        // continues so unblocked candidates still archive in the same run.
        let mut active_items = store.load_active()?;
        let mut recovered: Vec<Item> = Vec::new();
        let mut skipped_with_children: Vec<(String, Vec<String>)> = Vec::new();

        // Compute "done candidates" up-front so the child-set for each candidate
        // is evaluated against a stable snapshot of the remaining active items.
        let done_candidate_ids: Vec<String> = active_items
            .iter()
            .filter(|i| i.status == Status::Done)
            .map(|i| i.id.clone())
            .collect();

        for candidate_id in done_candidate_ids {
            // Children are non-done active items pointing at this candidate.
            // (Other done candidates also being archived in this sweep are not
            // blockers — they'll move to archive alongside their parent.)
            let children: Vec<String> = active_items
                .iter()
                .filter(|i| {
                    i.status != Status::Done && i.parent.as_deref() == Some(candidate_id.as_str())
                })
                .map(|i| i.id.clone())
                .collect();

            if !children.is_empty() {
                eprintln!(
                    "Warning: skipping {} — has active children: {}",
                    candidate_id,
                    children.join(", ")
                );
                skipped_with_children.push((candidate_id, children));
                continue;
            }

            // Pull the done item out and append to archive.
            let idx = active_items
                .iter()
                .position(|i| i.id == candidate_id)
                .expect("candidate collected from the same slice");
            let done_item = active_items.remove(idx);
            store.append_to_archive(&done_item)?;
            recovered.push(done_item);
        }

        if !recovered.is_empty() {
            store.save_active(&active_items)?;
        }

        // If at least one candidate existed and all were skipped, signal the
        // condition via a nonzero exit so scripts can detect it.
        let had_candidates = !recovered.is_empty() || !skipped_with_children.is_empty();
        let everything_blocked = had_candidates && recovered.is_empty();

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
                    task_golem::store::jsonl::append_to_archive(&pruned_path, pruned_item)?;
                }

                // Rewrite archive without pruned items
                task_golem::store::jsonl::write_atomic(&store.archive_path(), &keep)?;
            }
        }

        Ok(ArchiveResult {
            recovered_count: recovered.len(),
            recovered_ids: recovered.iter().map(|i| i.id.clone()).collect(),
            pruned_count: pruned.len(),
            pruned_ids: pruned.iter().map(|i| i.id.clone()).collect(),
            skipped_count: skipped_with_children.len(),
            skipped_ids: skipped_with_children
                .iter()
                .map(|(id, _)| id.clone())
                .collect(),
            everything_blocked,
        })
    })?;

    if json_mode {
        output::print_json(&serde_json::json!({
            "recovered": result.recovered_count,
            "recovered_ids": result.recovered_ids,
            "pruned": result.pruned_count,
            "pruned_ids": result.pruned_ids,
            "skipped": result.skipped_count,
            "skipped_ids": result.skipped_ids,
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
        if result.recovered_count == 0 && result.pruned_count == 0 && result.skipped_count == 0 {
            output::print_human("No items to archive or prune.");
        }
    }

    // Nonzero exit if every candidate was blocked by children — signal to
    // scripts that nothing was archived despite candidates existing.
    if result.everything_blocked {
        return Err(TgError::InvalidInput(format!(
            "All {} done candidate(s) skipped due to active children. Reparent or complete the children, then re-run `tg archive`.",
            result.skipped_count
        )));
    }

    Ok(())
}

struct ArchiveResult {
    recovered_count: usize,
    recovered_ids: Vec<String>,
    pruned_count: usize,
    pruned_ids: Vec<String>,
    skipped_count: usize,
    skipped_ids: Vec<String>,
    everything_blocked: bool,
}
