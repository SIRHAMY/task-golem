use std::collections::HashSet;

use chrono::Utc;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::deps;
use task_golem::model::extensions;
use task_golem::model::id;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::store::config::Config;
use task_golem::store::root;

pub fn run(
    json_mode: bool,
    title: String,
    description: Option<String>,
    priority: i64,
    dep_inputs: Vec<String>,
    tags: Vec<String>,
    sets: Vec<String>,
) -> Result<(), TgError> {
    Item::validate_title(&title)?;

    let project_dir = root::find_project_root_from_cwd()?;
    let config = Config::load(&project_dir)?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let archive_ids = store.load_archive_ids()?;

        // Collect all known IDs for collision check
        let mut all_ids: HashSet<String> = archive_ids.clone();
        for item in &items {
            all_ids.insert(item.id.clone());
        }

        let new_id = id::generate_id_with_prefix(&all_ids, &config.id_prefix, config.id_hex_len)?;

        // Build active-only ID set for validate_dep
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let active_id_set: HashSet<String> = active_ids.iter().cloned().collect();

        // Resolve and validate dependencies (with deduplication)
        let mut resolved_deps = Vec::new();
        for dep_input in &dep_inputs {
            let resolved = id::resolve_id(dep_input, &active_ids, &archive_ids, true)?;
            if resolved_deps.contains(&resolved) {
                continue; // skip duplicate deps
            }
            let warnings = deps::validate_dep(&new_id, &resolved, &active_id_set, &archive_ids)?;
            for w in &warnings {
                eprintln!("{}", w);
            }
            resolved_deps.push(resolved);
        }

        // Parse extension sets
        let mut ext = std::collections::BTreeMap::new();
        extensions::apply_sets(&mut ext, &sets)?;

        // Deduplicate tags
        let mut deduped_tags = Vec::new();
        for tag in tags {
            if !deduped_tags.contains(&tag) {
                deduped_tags.push(tag);
            }
        }

        let now = Utc::now();
        let item = Item {
            id: new_id,
            title,
            status: Status::Todo,
            priority,
            description,
            tags: deduped_tags,
            dependencies: resolved_deps,
            created_at: now,
            updated_at: now,
            blocked_reason: None,
            blocked_from_status: None,
            claimed_by: None,
            claimed_at: None,
            extensions: ext,
        };

        items.push(item.clone());
        store.save_active(&items)?;

        Ok(item)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!("Created item: {} - {}", item.id, item.title));
    }

    Ok(())
}
