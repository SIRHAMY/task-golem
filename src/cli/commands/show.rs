use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::id;
use task_golem::store::Store;
use task_golem::store::root;

pub fn run(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let items = store.load_active()?;
    let archive_ids = store.load_archive_ids()?;
    let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();

    // Resolve ID with active+archive scope
    let resolved_id = id::resolve_id(&id_input, &active_ids, &archive_ids, true)?;

    // Look in active items first
    if let Some(item) = items.into_iter().find(|i| i.id == resolved_id) {
        if json_mode {
            output::print_json(&item);
        } else {
            output::print_item_detail(&item);
        }
        return Ok(());
    }

    // Fallback to archive
    if let Some(item) = store.load_archive_item(&resolved_id)? {
        if json_mode {
            output::print_json(&item);
        } else {
            output::print_item_detail(&item);
        }
        return Ok(());
    }

    Err(TgError::ItemNotFound(id_input))
}
