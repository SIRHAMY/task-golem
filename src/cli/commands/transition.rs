use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::id;
use task_golem::model::status::Status;
use task_golem::store::Store;
use task_golem::store::root;

/// Result of a transition that may be idempotent.
enum TransitionResult {
    /// Transition was applied normally.
    Applied(Box<task_golem::model::item::Item>),
    /// Item was already in the target state — no-op.
    Idempotent(Status),
}

fn output_transition(json_mode: bool, result: &TransitionResult, action: &str) {
    match result {
        TransitionResult::Applied(item) => {
            if json_mode {
                output::print_json(item.as_ref());
            } else {
                output::print_human(&format!(
                    "{}: {} - {} [{}]",
                    action, item.id, item.title, item.status
                ));
            }
        }
        TransitionResult::Idempotent(status) => {
            if json_mode {
                output::print_json(&serde_json::json!({
                    "idempotent": true,
                    "previous_state": status.to_string(),
                }));
            } else {
                output::print_human(&format!("Already {} (no change)", status));
            }
        }
    }
}

/// Handle `tg do <id> [--claim <agent>]`
pub fn run_do(json_mode: bool, id_input: String, claim: Option<String>) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let result = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // Idempotent check: already doing
        if item.status == Status::Doing {
            if let Some(ref new_claim) = claim {
                if let Some(ref existing_claim) = item.claimed_by
                    && existing_claim != new_claim
                {
                    return Err(TgError::AlreadyClaimed(existing_claim.clone()));
                }
                // Set or re-claim: update claim fields
                item.claimed_by = Some(new_claim.clone());
                item.claimed_at = Some(chrono::Utc::now());
                item.updated_at = chrono::Utc::now();
                let result = item.clone();
                store.save_active(&items)?;
                return Ok(TransitionResult::Applied(Box::new(result)));
            }
            // Already doing with no claim argument — idempotent
            return Ok(TransitionResult::Idempotent(Status::Doing));
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
        Ok(TransitionResult::Applied(Box::new(result)))
    })?;

    output_transition(json_mode, &result, "Started");

    Ok(())
}

/// Handle `tg done <id>` — transitions to done and archives.
pub fn run_done(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let result = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let archive_ids = store.load_archive_ids()?;
        let resolved = id::resolve_id(&id_input, &active_ids, &archive_ids, true)?;

        // Check if item is in active store
        if let Some(idx) = items.iter().position(|i| i.id == resolved) {
            // Idempotent check: active item already done (shouldn't normally happen, but handle it)
            if items[idx].status == Status::Done {
                return Ok(TransitionResult::Idempotent(Status::Done));
            }

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

            Ok(TransitionResult::Applied(Box::new(done_item)))
        } else if archive_ids.contains(&resolved) {
            // Item is already in the archive — idempotent done
            Ok(TransitionResult::Idempotent(Status::Done))
        } else {
            Err(TgError::ItemNotFound(resolved))
        }
    })?;

    match &result {
        TransitionResult::Applied(item) => {
            if json_mode {
                output::print_json(item);
            } else {
                output::print_human(&format!("Done: {} - {} [archived]", item.id, item.title));
            }
        }
        TransitionResult::Idempotent(_) => {
            output_transition(json_mode, &result, "Done");
        }
    }

    Ok(())
}

/// Handle `tg todo <id>` — transitions back to todo, clears claims.
pub fn run_todo(json_mode: bool, id_input: String) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let result = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // Idempotent check: already todo
        if item.status == Status::Todo {
            return Ok(TransitionResult::Idempotent(Status::Todo));
        }

        if !item.status.can_transition_to(Status::Todo) {
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Todo,
            });
        }

        item.apply_todo();
        let result = item.clone();
        store.save_active(&items)?;
        Ok(TransitionResult::Applied(Box::new(result)))
    })?;

    match &result {
        TransitionResult::Applied(item) => {
            if json_mode {
                output::print_json(item);
            } else {
                output::print_human(&format!("Returned to todo: {} - {}", item.id, item.title));
            }
        }
        TransitionResult::Idempotent(_) => {
            output_transition(json_mode, &result, "Todo");
        }
    }

    Ok(())
}

/// Handle `tg block <id> [--reason <reason>]`
pub fn run_block(json_mode: bool, id_input: String, reason: Option<String>) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let result = store.with_lock(|store| {
        let mut items = store.load_active()?;
        let active_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let empty = std::collections::HashSet::new();
        let resolved = id::resolve_id(&id_input, &active_ids, &empty, false)?;

        let item = items
            .iter_mut()
            .find(|i| i.id == resolved)
            .ok_or_else(|| TgError::ItemNotFound(resolved.clone()))?;

        // Idempotent check: already blocked
        if item.status == Status::Blocked {
            return Ok(TransitionResult::Idempotent(Status::Blocked));
        }

        if !item.status.can_transition_to(Status::Blocked) {
            return Err(TgError::InvalidTransition {
                from: item.status,
                to: Status::Blocked,
            });
        }

        item.apply_block(reason);
        let result = item.clone();
        store.save_active(&items)?;
        Ok(TransitionResult::Applied(Box::new(result)))
    })?;

    match &result {
        TransitionResult::Applied(item) => {
            if json_mode {
                output::print_json(item);
            } else {
                output::print_human(&format!("Blocked: {} - {}", item.id, item.title));
            }
        }
        TransitionResult::Idempotent(_) => {
            output_transition(json_mode, &result, "Block");
        }
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
