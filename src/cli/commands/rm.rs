use serde::Serialize;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::id;
use task_golem::store::Store;
use task_golem::store::root;

#[derive(Debug, Serialize)]
struct RmOutput {
    removed: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cleared_deps_from: Vec<String>,
}

pub fn run(
    json_mode: bool,
    id_input: String,
    _force: bool,
    _clear_deps: bool,
) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let rm_output = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let archive_ids = store.load_archive_ids()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

        // Resolve ID (active-only scope)
        let resolved_id = id::resolve_id(&id_input, &active_ids, &archive_ids, false)?;

        // Reject if this item has active children (not overridable by --force
        // since orphaning children is a destructive invariant violation — user
        // must explicitly reparent or delete children first).
        let children: Vec<String> = items
            .iter()
            .filter(|i| i.parent.as_deref() == Some(resolved_id.as_str()))
            .map(|i| i.id.clone())
            .collect();
        if !children.is_empty() {
            return Err(TgError::ParentHasChildren {
                id: resolved_id,
                children,
            });
        }

        // Check for dependents
        let dependents = task_golem::model::deps::active_dependents(&items, &resolved_id);

        if !dependents.is_empty() {
            return Err(TgError::DependentExists(
                resolved_id.clone(),
                format!(
                    "{}. Remove those dependency edges first",
                    dependents.join(", ")
                ),
            ));
        }

        // Remove the item
        items.retain(|i| i.id != resolved_id);
        store.save_active(&items)?;

        Ok(RmOutput {
            removed: resolved_id,
            cleared_deps_from: Vec::new(),
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
