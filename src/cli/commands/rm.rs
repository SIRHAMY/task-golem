use serde::Serialize;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::id;
use task_golem::store::root;
use task_golem::store::Store;

#[derive(Debug, Serialize)]
struct RmOutput {
    removed: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cleared_deps_from: Vec<String>,
}

pub fn run(
    json_mode: bool,
    id_input: String,
    force: bool,
    clear_deps: bool,
) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let rm_output = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let archive_ids = store.load_archive_ids()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

        // Resolve ID (active-only scope)
        let resolved_id = id::resolve_id(&id_input, &active_ids, &archive_ids, false)?;

        // Check for dependents
        let dependents: Vec<String> = items
            .iter()
            .filter(|i| i.id != resolved_id && i.dependencies.contains(&resolved_id))
            .map(|i| i.id.clone())
            .collect();

        if !dependents.is_empty() && !force {
            return Err(TgError::DependentExists(
                resolved_id.clone(),
                format!(
                    "{}. Use --force to remove anyway, or --force --clear-deps to also remove this ID from dependents' dep lists",
                    dependents.join(", ")
                ),
            ));
        }

        let mut cleared_from = Vec::new();
        if force && clear_deps && !dependents.is_empty() {
            // Clear this ID from all dependents
            for item in &mut items {
                if item.dependencies.contains(&resolved_id) {
                    item.dependencies.retain(|d| d != &resolved_id);
                    cleared_from.push(item.id.clone());
                }
            }
        }

        // Remove the item
        items.retain(|i| i.id != resolved_id);
        store.save_active(&items)?;

        Ok(RmOutput {
            removed: resolved_id,
            cleared_deps_from: cleared_from,
        })
    })?;

    if json_mode {
        output::print_json(&rm_output);
    } else {
        let mut msg = format!("Removed item: {}", rm_output.removed);
        if !rm_output.cleared_deps_from.is_empty() {
            msg.push_str(&format!(
                " (cleared deps from: {})",
                rm_output.cleared_deps_from.join(", ")
            ));
        }
        output::print_human(&msg);
    }

    Ok(())
}
