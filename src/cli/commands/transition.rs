use crate::cli::output;
use crate::errors::TgError;
use crate::model::id;
use crate::model::status::Status;
use crate::store::root;
use crate::store::Store;

/// Handle `tg do <id> [--claim <agent>]`
pub fn run_do(json_mode: bool, id_input: String, claim: Option<String>) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // If already doing and a claim is provided, check for claim conflicts
        if item.status == Status::Doing {
            if let Some(ref new_claim) = claim
                && let Some(ref existing_claim) = item.claimed_by
            {
                if existing_claim != new_claim {
                    return Err(TgError::AlreadyClaimed(existing_claim.clone()));
                }
                // Same agent re-claim: update claimed_at
                item.claimed_at = Some(chrono::Utc::now());
                item.updated_at = chrono::Utc::now();
                let result = item.clone();
                store.save_active(&items)?;
                return Ok(result);
            }
            // No claim or no existing claim: invalid transition doing→doing
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Doing,
            });
        }

        // Validate transition for non-doing states
        if !item.status.can_transition_to(Status::Doing) {
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Doing,
            });
        }

        item.apply_do(claim);
        let result = item.clone();
        store.save_active(&items)?;
        Ok(result)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!(
            "Started: {} - {} [doing]",
            item.id, item.title
        ));
    }

    Ok(())
}

/// Handle `tg done <id>` — transitions to done and archives.
pub fn run_done(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let idx = items
            .iter()
            .position(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // Validate transition
        if !items[idx].status.can_transition_to(Status::Done) {
            return Err(TgError::InvalidTransition {
                from: items[idx].status,
                to: Status::Done,
            });
        }

        items[idx].apply_done();
        let done_item = items[idx].clone();

        // Archive-first write ordering: append to archive, then remove from active.
        // Failure after archive append but before active rewrite = benign duplicate.
        store.append_to_archive(&done_item)?;

        items.remove(idx);
        store.save_active(&items)?;

        Ok(done_item)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!(
            "Done: {} - {} [archived]",
            item.id, item.title
        ));
    }

    Ok(())
}

/// Handle `tg todo <id>` — transitions back to todo, clears claims.
pub fn run_todo(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        if !item.status.can_transition_to(Status::Todo) {
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Todo,
            });
        }

        item.apply_todo();
        let result = item.clone();
        store.save_active(&items)?;
        Ok(result)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!("Returned to todo: {} - {}", item.id, item.title));
    }

    Ok(())
}

/// Handle `tg block <id> [--reason <reason>]`
pub fn run_block(
    json_mode: bool,
    id_input: String,
    reason: Option<String>,
) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        if !item.status.can_transition_to(Status::Blocked) {
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Blocked,
            });
        }

        item.apply_block(reason);
        let result = item.clone();
        store.save_active(&items)?;
        Ok(result)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!("Blocked: {} - {}", item.id, item.title));
    }

    Ok(())
}

/// Handle `tg unblock <id>`
pub fn run_unblock(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let item = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // Unblock only works on Blocked items
        if item.status != Status::Blocked {
            return Err(TgError::InvalidInput(format!(
                "Cannot unblock: item status is '{}', expected 'blocked'",
                item.status
            )));
        }

        item.apply_unblock();
        let result = item.clone();
        store.save_active(&items)?;
        Ok(result)
    })?;

    if json_mode {
        output::print_json(&item);
    } else {
        output::print_human(&format!(
            "Unblocked: {} - {} [{}]",
            item.id, item.title, item.status
        ));
    }

    Ok(())
}
