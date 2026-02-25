use crate::cli::output;
use crate::errors::TgError;
use crate::model::id;
use crate::store::root;
use crate::store::Store;

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
            print_item_human(&item);
        }
        return Ok(());
    }

    // Fallback to archive
    if let Some(item) = store.load_archive_item(&resolved_id)? {
        if json_mode {
            output::print_json(&item);
        } else {
            print_item_human(&item);
        }
        return Ok(());
    }

    Err(TgError::ItemNotFound(id_input))
}

fn print_item_human(item: &crate::model::item::Item) {
    output::print_human(&format!("ID:           {}", item.id));
    output::print_human(&format!("Title:        {}", item.title));
    output::print_human(&format!("Status:       {}", item.status));
    output::print_human(&format!("Priority:     {}", item.priority));
    if let Some(ref desc) = item.description {
        output::print_human(&format!("Description:  {}", desc));
    }
    if !item.tags.is_empty() {
        output::print_human(&format!("Tags:         {}", item.tags.join(", ")));
    }
    if !item.dependencies.is_empty() {
        output::print_human(&format!(
            "Dependencies: {}",
            item.dependencies.join(", ")
        ));
    }
    output::print_human(&format!("Created:      {}", item.created_at));
    output::print_human(&format!("Updated:      {}", item.updated_at));
    if let Some(ref by) = item.claimed_by {
        output::print_human(&format!("Claimed by:   {}", by));
    }
    if let Some(ref reason) = item.blocked_reason {
        output::print_human(&format!("Blocked:      {}", reason));
    }
    for (key, value) in &item.extensions {
        output::print_human(&format!("{}:  {}", key, value));
    }
}
