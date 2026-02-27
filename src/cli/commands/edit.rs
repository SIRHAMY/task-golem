use chrono::Utc;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::deps;
use task_golem::model::extensions;
use task_golem::model::id;
use task_golem::store::root;
use task_golem::store::Store;

#[allow(clippy::too_many_arguments)]
pub fn run(
    json_mode: bool,
    id_input: String,
    title: Option<String>,
    priority: Option<i64>,
    description: Option<String>,
    add_deps: Vec<String>,
    rm_deps: Vec<String>,
    add_tags: Vec<String>,
    rm_tags: Vec<String>,
    sets: Vec<String>,
) -> Result<(), TgError> {
    // Validate title if provided
    if let Some(ref t) = title {
        task_golem::model::item::Item::validate_title(t)?;
    }

    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let updated_item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let archive_ids = store.load_archive_ids()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

        // Resolve ID (active-only scope for editing)
        let resolved_id = id::resolve_id(&id_input, &active_ids, &archive_ids, false)?;

        let item_idx = items
            .iter()
            .position(|i| i.id == resolved_id)
            .ok_or_else(|| TgError::ItemNotFound(id_input.clone()))?;

        // Apply field changes
        if let Some(new_title) = title {
            items[item_idx].title = new_title;
        }
        if let Some(new_priority) = priority {
            items[item_idx].priority = new_priority;
        }
        if let Some(new_desc) = description {
            items[item_idx].description = Some(new_desc);
        }

        // Build active_ids set for validation
        let active_id_set: std::collections::HashSet<String> =
            active_ids.iter().cloned().collect();

        // Process dep removals first
        for rm_dep in &rm_deps {
            let resolved_dep = id::resolve_id(rm_dep, &active_ids, &archive_ids, true)?;
            items[item_idx].dependencies.retain(|d| d != &resolved_dep);
        }

        // Process dep additions
        for add_dep in &add_deps {
            let resolved_dep = id::resolve_id(add_dep, &active_ids, &archive_ids, true)?;
            let warnings = deps::validate_dep(
                &resolved_id,
                &resolved_dep,
                &active_id_set,
                &archive_ids,
            )?;
            for w in &warnings {
                eprintln!("{}", w);
            }

            // Temporarily add the dep to check for cycles
            if !items[item_idx].dependencies.contains(&resolved_dep) {
                items[item_idx].dependencies.push(resolved_dep.clone());
                if deps::would_create_cycle(&items, &resolved_id, &resolved_dep) {
                    items[item_idx].dependencies.pop();
                    return Err(TgError::CycleDetected(format!(
                        "Adding dependency {} -> {} would create a cycle",
                        resolved_id, resolved_dep
                    )));
                }
            }
        }

        // Process tag changes
        for tag in &add_tags {
            if !items[item_idx].tags.contains(tag) {
                items[item_idx].tags.push(tag.clone());
            }
        }
        for tag in &rm_tags {
            items[item_idx].tags.retain(|t| t != tag);
        }

        // Apply extension changes
        extensions::apply_sets(&mut items[item_idx].extensions, &sets)?;

        // Update timestamp
        items[item_idx].updated_at = Utc::now();

        let updated = items[item_idx].clone();
        store.save_active(&items)?;

        Ok(updated)
    })?;

    if json_mode {
        output::print_json(&updated_item);
    } else {
        output::print_human(&format!(
            "Updated item: {} - {}",
            updated_item.id, updated_item.title
        ));
    }

    Ok(())
}
