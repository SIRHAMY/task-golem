use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use super::instantiate::WorkflowMetadata;
use super::template::{ContextMode, NodeKind};
use crate::errors::TgError;
use crate::model::deps;
use crate::model::item::Item;
use crate::model::status::Status;
use crate::store::Store;

const WORKFLOW_EXTENSION: &str = "x-workflow";
const WORKFLOW_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowRunKind {
    Resume,
    Claimed,
    Complete,
}

impl std::fmt::Display for WorkflowRunKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Resume => "resume",
            Self::Claimed => "claimed",
            Self::Complete => "complete",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowRunOutcome {
    pub version: u32,
    pub outcome: WorkflowRunKind,
    pub campaign_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Clone)]
struct ScopedItem {
    item: Item,
    metadata: WorkflowMetadata,
    session_ref: Option<String>,
    is_archived: bool,
}

struct HierarchyFacts {
    children: HashMap<String, Vec<String>>,
    depth: HashMap<String, usize>,
    preorder: HashMap<String, usize>,
    subtree_end: HashMap<String, usize>,
    has_task_descendant: HashSet<String>,
    has_active_task_descendant: HashSet<String>,
    has_active_descendant: HashSet<String>,
}

struct CampaignProjection {
    items: BTreeMap<String, ScopedItem>,
    hierarchy: HierarchyFacts,
}

enum SelectedTask {
    Rollup,
    Resume(String),
    Claim(String),
    Complete,
}

pub fn run_campaign_state_shell(
    store: &Store,
    campaign_id: &str,
) -> Result<WorkflowRunOutcome, TgError> {
    let campaign_id = campaign_id.to_string();
    loop {
        let (active, archive) = load_snapshot(store)?;
        let projection = project_campaign(&active, &archive, &campaign_id)?;
        match select_task(&projection, &campaign_id)? {
            SelectedTask::Rollup => {
                close_eligible_containers(store, &campaign_id)?;
            }
            SelectedTask::Resume(task_id) => {
                let (active, archive) = load_snapshot(store)?;
                let projection = project_campaign(&active, &archive, &campaign_id)?;
                confirm_claimed_task(&projection, &task_id, &campaign_id)?;
                return Ok(outcome(WorkflowRunKind::Resume, campaign_id, Some(task_id)));
            }
            SelectedTask::Claim(_) => {
                let claimed_task_id = store.with_lock(|store| {
                    let (mut active, archive) = load_snapshot(store)?;
                    let projection = project_campaign(&active, &archive, &campaign_id)?;
                    match select_task(&projection, &campaign_id)? {
                        SelectedTask::Claim(task_id) => {
                            claim_task(store, &mut active, &task_id, &campaign_id)?;
                            Ok(Some(task_id))
                        }
                        _ => Ok(None),
                    }
                })?;
                let Some(task_id) = claimed_task_id else {
                    continue;
                };
                let (active, archive) = load_snapshot(store)?;
                let projection = project_campaign(&active, &archive, &campaign_id)?;
                confirm_claimed_task(&projection, &task_id, &campaign_id)?;
                return Ok(outcome(
                    WorkflowRunKind::Claimed,
                    campaign_id,
                    Some(task_id),
                ));
            }
            SelectedTask::Complete => {
                return Ok(outcome(WorkflowRunKind::Complete, campaign_id, None));
            }
        }
    }
}

fn outcome(
    outcome: WorkflowRunKind,
    campaign_id: String,
    task_id: Option<String>,
) -> WorkflowRunOutcome {
    WorkflowRunOutcome {
        version: WORKFLOW_VERSION,
        outcome,
        campaign_id,
        task_id,
    }
}

fn load_snapshot(store: &Store) -> Result<(Vec<Item>, Vec<Item>), TgError> {
    Ok((store.load_active()?, store.load_all_archive_strict()?))
}

fn project_campaign(
    active: &[Item],
    archive: &[Item],
    campaign_id: &str,
) -> Result<CampaignProjection, TgError> {
    let mut all_items = HashMap::with_capacity(active.len() + archive.len());
    let mut all_children = HashMap::<&str, Vec<&str>>::new();
    for (item, is_archived) in active
        .iter()
        .map(|item| (item, false))
        .chain(archive.iter().map(|item| (item, true)))
    {
        if all_items
            .insert(item.id.as_str(), (item, is_archived))
            .is_some()
        {
            return corrupt(format!(
                "duplicate item ID '{}' in active/archive records",
                item.id
            ));
        }
        if let Some(parent_id) = item.parent.as_deref() {
            all_children
                .entry(parent_id)
                .or_default()
                .push(item.id.as_str());
        }
    }
    let Some((campaign, _)) = all_items.get(campaign_id) else {
        return Err(TgError::InvalidInput(format!(
            "workflow campaign '{campaign_id}' was not found"
        )));
    };
    let campaign_metadata = parse_metadata(campaign)?;
    if campaign.parent.is_some() || campaign_metadata.metadata.kind != NodeKind::Container {
        return corrupt(format!(
            "workflow campaign '{campaign_id}' must be a root container"
        ));
    }

    let mut scope_order = Vec::new();
    let mut depth = HashMap::new();
    let mut preorder = HashMap::new();
    let mut subtree_end = HashMap::new();
    let mut seen = HashSet::new();
    let mut pending = vec![(campaign_id, 0, false)];
    while let Some((item_id, item_depth, exiting)) = pending.pop() {
        if exiting {
            subtree_end.insert(item_id.to_string(), scope_order.len());
            continue;
        }
        if !seen.insert(item_id) {
            return corrupt(format!(
                "workflow Campaign '{campaign_id}' has a parent cycle at '{item_id}'"
            ));
        }
        preorder.insert(item_id.to_string(), scope_order.len());
        depth.insert(item_id.to_string(), item_depth);
        scope_order.push(item_id.to_string());
        pending.push((item_id, item_depth, true));
        if let Some(children) = all_children.get(item_id) {
            for child_id in children.iter().rev() {
                pending.push((child_id, item_depth + 1, false));
            }
        }
    }

    let mut items = BTreeMap::new();
    let mut children = HashMap::<String, Vec<String>>::new();
    for item_id in &scope_order {
        let (item, is_archived) = all_items[item_id.as_str()];
        let parsed_metadata = parse_metadata(item)?;
        if let Some(parent_id) = item.parent.as_deref() {
            children
                .entry(parent_id.to_string())
                .or_default()
                .push(item_id.clone());
        }
        items.insert(
            item_id.clone(),
            ScopedItem {
                item: item.clone(),
                metadata: parsed_metadata.metadata,
                session_ref: parsed_metadata.session_ref,
                is_archived,
            },
        );
    }
    let mut has_task_descendant = HashSet::new();
    let mut has_active_task_descendant = HashSet::new();
    let mut has_active_descendant = HashSet::new();
    for item_id in scope_order.iter().rev() {
        let child_ids = children.get(item_id).map(Vec::as_slice).unwrap_or(&[]);
        for child_id in child_ids {
            let child = &items[child_id];
            if !child.is_archived || has_active_descendant.contains(child_id) {
                has_active_descendant.insert(item_id.clone());
            }
            if child.metadata.kind == NodeKind::Task || has_task_descendant.contains(child_id) {
                has_task_descendant.insert(item_id.clone());
            }
            if (child.metadata.kind == NodeKind::Task && !child.is_archived)
                || has_active_task_descendant.contains(child_id)
            {
                has_active_task_descendant.insert(item_id.clone());
            }
        }
    }
    let projection = CampaignProjection {
        items,
        hierarchy: HierarchyFacts {
            children,
            depth,
            preorder,
            subtree_end,
            has_task_descendant,
            has_active_task_descendant,
            has_active_descendant,
        },
    };
    validate_campaign(&projection, campaign_id, &campaign_metadata.metadata)?;

    for (item_id, (item, _)) in &all_items {
        if projection.items.contains_key(*item_id) {
            continue;
        }
        let Some(metadata) = item.extensions.get(WORKFLOW_EXTENSION) else {
            continue;
        };
        if metadata
            .as_object()
            .and_then(|fields| fields.get("instance"))
            .and_then(serde_json::Value::as_str)
            == Some(campaign_metadata.metadata.instance.as_str())
        {
            return corrupt(format!(
                "workflow item '{}' with Campaign instance '{}' is outside Campaign '{}'",
                item.id, campaign_metadata.metadata.instance, campaign_id
            ));
        }
    }
    Ok(projection)
}

fn validate_campaign(
    projection: &CampaignProjection,
    campaign_id: &str,
    campaign_metadata: &WorkflowMetadata,
) -> Result<(), TgError> {
    for parent_id in projection.hierarchy.children.keys() {
        if !projection.items.contains_key(parent_id) {
            return corrupt(format!(
                "workflow Campaign '{campaign_id}' has an out-of-scope parent '{parent_id}'"
            ));
        }
    }
    let mut node_ids = HashMap::new();
    let mut shared_contexts = HashMap::<String, (String, Vec<String>)>::new();
    for scoped in projection.items.values() {
        validate_metadata(scoped)?;
        if scoped.metadata.version != campaign_metadata.version
            || scoped.metadata.instance != campaign_metadata.instance
            || scoped.metadata.instance_digest != campaign_metadata.instance_digest
        {
            return corrupt(format!(
                "workflow item '{}' has different Campaign identity",
                scoped.item.id
            ));
        }
        if node_ids
            .insert(scoped.metadata.node.as_str(), scoped.item.id.as_str())
            .is_some()
        {
            return corrupt(format!(
                "workflow Campaign '{campaign_id}' has duplicate node '{}'",
                scoped.metadata.node
            ));
        }
        if scoped.metadata.kind == NodeKind::Task
            && scoped.metadata.context.as_ref().unwrap().mode == ContextMode::Shared
        {
            let context = scoped.metadata.context.as_ref().unwrap();
            let plugin = scoped.metadata.plugin.as_ref().unwrap().argv.join("\0");
            let entry = shared_contexts
                .entry(context.key.clone().unwrap())
                .or_insert_with(|| (plugin.clone(), Vec::new()));
            if entry.0 != plugin {
                return corrupt(format!(
                    "shared context '{}' is invalid",
                    context.key.as_ref().unwrap()
                ));
            }
            entry.1.push(scoped.item.id.clone());
        }
    }

    for scoped in projection.items.values() {
        if scoped.is_archived {
            if scoped.item.status != Status::Done
                || scoped.item.claimed_by.is_some()
                || scoped.item.claimed_at.is_some()
            {
                return corrupt(format!(
                    "archived workflow item '{}' is not an unclaimed done item",
                    scoped.item.id
                ));
            }
        } else if scoped.item.status == Status::Done {
            return corrupt(format!(
                "active workflow item '{}' is done instead of archived",
                scoped.item.id
            ));
        }
        match scoped.metadata.kind {
            NodeKind::Container => {
                if !scoped.item.dependencies.is_empty() {
                    return corrupt(format!(
                        "workflow container '{}' has dependencies",
                        scoped.item.id
                    ));
                }
                if !scoped.is_archived
                    && (scoped.item.status != Status::Todo
                        || scoped.item.claimed_by.is_some()
                        || scoped.item.claimed_at.is_some())
                {
                    return corrupt(format!(
                        "active workflow container '{}' has dispatch state",
                        scoped.item.id
                    ));
                }
                if scoped.is_archived
                    && projection
                        .hierarchy
                        .has_active_descendant
                        .contains(&scoped.item.id)
                {
                    return corrupt(format!(
                        "archived workflow container '{}' has an active descendant",
                        scoped.item.id
                    ));
                }
            }
            NodeKind::Task => {
                validate_task_state(scoped)?;
                let mut dependencies = HashSet::new();
                for dependency_id in &scoped.item.dependencies {
                    if !dependencies.insert(dependency_id) {
                        return corrupt(format!(
                            "workflow task '{}' has duplicate dependency '{}'",
                            scoped.item.id, dependency_id
                        ));
                    }
                    let dependency = projection.items.get(dependency_id).ok_or_else(|| {
                        TgError::StorageCorruption(format!(
                            "workflow task '{}' has out-of-scope dependency '{}'",
                            scoped.item.id, dependency_id
                        ))
                    })?;
                    if dependency.metadata.kind != NodeKind::Task {
                        return corrupt(format!(
                            "workflow task '{}' depends on non-task '{}'",
                            scoped.item.id, dependency_id
                        ));
                    }
                }
            }
        }
        if scoped.item.id == campaign_id {
            continue;
        }
        let parent_id = scoped.item.parent.as_deref().ok_or_else(|| {
            TgError::StorageCorruption(format!(
                "workflow item '{}' is outside Campaign '{}'",
                scoped.item.id, campaign_id
            ))
        })?;
        let parent = projection.items.get(parent_id).ok_or_else(|| {
            TgError::StorageCorruption(format!(
                "workflow item '{}' has an out-of-scope parent '{}'",
                scoped.item.id, parent_id
            ))
        })?;
        if parent.metadata.kind != NodeKind::Container {
            return corrupt(format!(
                "workflow item '{}' parent '{}' is not a container",
                scoped.item.id, parent_id
            ));
        }
    }

    let scoped_items = projection
        .items
        .values()
        .map(|item| item.item.clone())
        .collect::<Vec<_>>();
    if !deps::detect_all_cycles(&scoped_items).is_empty() {
        return corrupt(format!(
            "workflow Campaign '{campaign_id}' has a dependency cycle"
        ));
    }
    let shared_context_owners = shared_contexts.keys().cloned().collect::<HashSet<_>>();
    for (key, (_, task_ids)) in shared_contexts {
        let Some(owner_id) = node_ids.get(key.as_str()) else {
            return corrupt(format!("shared context '{key}' has no owner"));
        };
        let owner = &projection.items[*owner_id];
        if owner.metadata.kind != NodeKind::Container {
            return corrupt(format!("shared context '{key}' is invalid"));
        }
        for task_id in task_ids {
            if !is_ancestor(projection, owner_id, &task_id) {
                return corrupt(format!(
                    "shared context '{key}' is not an ancestor of task '{task_id}'"
                ));
            }
        }
    }
    for scoped in projection.items.values() {
        if scoped.session_ref.is_some()
            && (scoped.metadata.kind != NodeKind::Container
                || !shared_context_owners.contains(&scoped.metadata.node))
        {
            return corrupt(format!(
                "workflow item '{}' stores session_ref outside a shared-context owner",
                scoped.item.id
            ));
        }
        if scoped.metadata.kind == NodeKind::Container
            && !projection
                .hierarchy
                .has_task_descendant
                .contains(&scoped.item.id)
        {
            return corrupt(format!(
                "workflow container '{}' has no Task descendant",
                scoped.item.id
            ));
        }
    }
    Ok(())
}

fn parse_metadata(item: &Item) -> Result<ParsedMetadata, TgError> {
    let metadata = item.extensions.get(WORKFLOW_EXTENSION).ok_or_else(|| {
        TgError::StorageCorruption(format!(
            "workflow item '{}' has no x-workflow metadata",
            item.id
        ))
    })?;
    let mut immutable = metadata.clone();
    let session_ref = immutable
        .as_object_mut()
        .and_then(|fields| fields.remove("session_ref"));
    let metadata = serde_json::from_value(immutable).map_err(|error| {
        TgError::StorageCorruption(format!(
            "invalid x-workflow metadata for item '{}': {error}",
            item.id
        ))
    })?;
    let session_ref = match session_ref {
        None => None,
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => Some(value),
        Some(_) => {
            return corrupt(format!(
                "workflow item '{}' has invalid session_ref",
                item.id
            ));
        }
    };
    Ok(ParsedMetadata {
        metadata,
        session_ref,
    })
}

struct ParsedMetadata {
    metadata: WorkflowMetadata,
    session_ref: Option<String>,
}

fn validate_metadata(scoped: &ScopedItem) -> Result<(), TgError> {
    let metadata = &scoped.metadata;
    if metadata.version != WORKFLOW_VERSION
        || metadata.instance.trim().is_empty()
        || metadata.node.trim().is_empty()
        || metadata.instance_digest.trim().is_empty()
    {
        return corrupt(format!(
            "workflow item '{}' has incomplete identity metadata",
            scoped.item.id
        ));
    }
    match metadata.kind {
        NodeKind::Container => {
            if metadata.plugin.is_some()
                || metadata.context.is_some()
                || metadata.input.is_some()
                || metadata.verify.is_some()
            {
                return corrupt(format!(
                    "workflow container '{}' has task execution metadata",
                    scoped.item.id
                ));
            }
        }
        NodeKind::Task => {
            if scoped.session_ref.is_some() {
                return corrupt(format!(
                    "workflow task '{}' stores a session_ref",
                    scoped.item.id
                ));
            }
            let plugin = metadata.plugin.as_ref().ok_or_else(|| {
                TgError::StorageCorruption(format!(
                    "workflow task '{}' has no plugin metadata",
                    scoped.item.id
                ))
            })?;
            let context = metadata.context.as_ref().ok_or_else(|| {
                TgError::StorageCorruption(format!(
                    "workflow task '{}' has no context metadata",
                    scoped.item.id
                ))
            })?;
            if plugin.version != WORKFLOW_VERSION
                || plugin
                    .argv
                    .first()
                    .is_none_or(|program| program.trim().is_empty())
                || plugin.argv.iter().any(|argument| argument.contains('\0'))
                || metadata.input.is_none()
                || metadata.verify.as_ref().is_some_and(|argv| {
                    argv.is_empty()
                        || argv[0].trim().is_empty()
                        || argv.iter().any(|argument| argument.contains('\0'))
                })
            {
                return corrupt(format!(
                    "workflow task '{}' has malformed execution metadata",
                    scoped.item.id
                ));
            }
            match context.mode {
                ContextMode::Shared
                    if context
                        .key
                        .as_deref()
                        .is_some_and(|key| !key.trim().is_empty()) => {}
                ContextMode::Fresh | ContextMode::None if context.key.is_none() => {}
                _ => {
                    return corrupt(format!(
                        "workflow task '{}' has malformed context",
                        scoped.item.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_task_state(scoped: &ScopedItem) -> Result<(), TgError> {
    if scoped.is_archived || scoped.item.status == Status::Doing {
        return Ok(());
    }
    if scoped.item.claimed_by.is_some() || scoped.item.claimed_at.is_some() {
        return corrupt(format!(
            "workflow task '{}' has a claim outside doing state",
            scoped.item.id
        ));
    }
    Ok(())
}

fn is_ancestor(projection: &CampaignProjection, ancestor_id: &str, item_id: &str) -> bool {
    let Some(&ancestor_start) = projection.hierarchy.preorder.get(ancestor_id) else {
        return false;
    };
    let Some(&item_start) = projection.hierarchy.preorder.get(item_id) else {
        return false;
    };
    let Some(&ancestor_end) = projection.hierarchy.subtree_end.get(ancestor_id) else {
        return false;
    };
    ancestor_id != item_id && ancestor_start < item_start && item_start < ancestor_end
}

fn select_task(
    projection: &CampaignProjection,
    campaign_id: &str,
) -> Result<SelectedTask, TgError> {
    if projection.items.values().all(|item| item.is_archived) {
        return Ok(SelectedTask::Complete);
    }
    if !eligible_container_rollup_ids(projection).is_empty() {
        return Ok(SelectedTask::Rollup);
    }
    let claim = format!("workflow:{campaign_id}");
    let doing = projection
        .items
        .values()
        .filter(|item| {
            !item.is_archived
                && item.metadata.kind == NodeKind::Task
                && item.item.status == Status::Doing
        })
        .collect::<Vec<_>>();
    if doing.len() > 1
        || doing.first().is_some_and(|item| {
            item.item.claimed_by.as_deref() != Some(claim.as_str())
                || item.item.claimed_at.is_none()
        })
    {
        return Err(TgError::InvalidInput(format!(
            "workflow campaign '{campaign_id}' has ambiguous doing Tasks"
        )));
    }
    if let Some(task) = doing.first() {
        return Ok(SelectedTask::Resume(task.item.id.clone()));
    }
    let mut ready = projection
        .items
        .values()
        .filter(|item| {
            !item.is_archived
                && item.metadata.kind == NodeKind::Task
                && item.item.status == Status::Todo
                && dependencies_are_archived(projection, item)
        })
        .collect::<Vec<_>>();
    ready.sort_by(|left, right| {
        right
            .item
            .priority
            .cmp(&left.item.priority)
            .then_with(|| left.item.created_at.cmp(&right.item.created_at))
            .then_with(|| left.item.id.cmp(&right.item.id))
    });
    ready
        .first()
        .map(|item| SelectedTask::Claim(item.item.id.clone()))
        .ok_or_else(|| {
            TgError::InvalidInput(format!(
                "workflow campaign '{campaign_id}' has no ready Task"
            ))
        })
}

fn eligible_container_rollup_ids(projection: &CampaignProjection) -> Vec<String> {
    let mut candidates = projection
        .items
        .values()
        .filter(|item| {
            !item.is_archived
                && item.metadata.kind == NodeKind::Container
                && projection
                    .hierarchy
                    .has_task_descendant
                    .contains(&item.item.id)
                && !projection
                    .hierarchy
                    .has_active_task_descendant
                    .contains(&item.item.id)
        })
        .map(|item| item.item.id.clone())
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        projection.hierarchy.depth[right]
            .cmp(&projection.hierarchy.depth[left])
            .then_with(|| left.cmp(right))
    });
    candidates
}

fn claim_task(
    store: &Store,
    active: &mut [Item],
    task_id: &str,
    campaign_id: &str,
) -> Result<(), TgError> {
    let task = active
        .iter_mut()
        .find(|item| item.id == task_id)
        .ok_or_else(|| {
            TgError::StorageCorruption(format!("selected Task '{task_id}' disappeared"))
        })?;
    if task.status != Status::Todo || task.claimed_by.is_some() || task.claimed_at.is_some() {
        return corrupt(format!("selected Task '{task_id}' is no longer claimable"));
    }
    let change = task.apply_do(Some(format!("workflow:{campaign_id}")));
    store.commit_status_change(active, change)
}

fn confirm_claimed_task(
    projection: &CampaignProjection,
    task_id: &str,
    campaign_id: &str,
) -> Result<(), TgError> {
    let task = projection.items.get(task_id).ok_or_else(|| {
        TgError::StorageCorruption(format!("selected Task '{task_id}' is outside Campaign"))
    })?;
    if task.is_archived
        || task.metadata.kind != NodeKind::Task
        || task.item.status != Status::Doing
        || task.item.claimed_by.as_deref() != Some(format!("workflow:{campaign_id}").as_str())
        || task.item.claimed_at.is_none()
        || !dependencies_are_archived(projection, task)
    {
        return corrupt(format!(
            "selected Task '{task_id}' failed claim revalidation"
        ));
    }
    Ok(())
}

fn dependencies_are_archived(projection: &CampaignProjection, task: &ScopedItem) -> bool {
    task.item.dependencies.iter().all(|dependency_id| {
        projection
            .items
            .get(dependency_id)
            .is_some_and(|dependency| dependency.is_archived)
    })
}

fn close_eligible_containers(store: &Store, campaign_id: &str) -> Result<(), TgError> {
    store.with_lock(|store| {
        let (mut active, archive) = load_snapshot(store)?;
        let projection = project_campaign(&active, &archive, campaign_id)?;
        let candidates = eligible_container_rollup_ids(&projection);
        if candidates.is_empty() {
            return Ok(());
        }
        let positions = active
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let candidate_ids = candidates.iter().cloned().collect::<HashSet<_>>();
        let mut done_items = Vec::with_capacity(candidates.len());
        let mut changes = Vec::with_capacity(candidates.len());
        for candidate_id in &candidates {
            let index = positions.get(candidate_id).ok_or_else(|| {
                TgError::StorageCorruption(format!(
                    "container '{candidate_id}' disappeared during rollup"
                ))
            })?;
            let item = &mut active[*index];
            changes.push(item.apply_done());
            done_items.push(item.clone());
        }
        active.retain(|item| !candidate_ids.contains(&item.id));
        store.commit_done_batch(&active, &done_items, changes)
    })
}

fn corrupt<T>(message: impl Into<String>) -> Result<T, TgError> {
    Err(TgError::StorageCorruption(message.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::workflow::instantiate::WorkflowPlugin;
    use crate::workflow::template::WorkflowContext;

    fn workflow_item(id: String, parent: Option<String>, kind: NodeKind) -> Item {
        let metadata = WorkflowMetadata {
            version: WORKFLOW_VERSION,
            instance: "deep".to_string(),
            node: id.clone(),
            kind,
            plugin: (kind == NodeKind::Task).then(|| WorkflowPlugin {
                version: WORKFLOW_VERSION,
                argv: vec!["worker".to_string()],
            }),
            context: (kind == NodeKind::Task).then(|| WorkflowContext {
                mode: ContextMode::Fresh,
                key: None,
            }),
            input: (kind == NodeKind::Task).then(BTreeMap::new),
            verify: None,
            instance_digest: "sha256:deep".to_string(),
        };
        Item {
            id,
            title: "deep".to_string(),
            status: Status::Todo,
            priority: 0,
            description: None,
            tags: vec![],
            dependencies: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            blocked_reason: None,
            blocked_from_status: None,
            claimed_by: None,
            claimed_at: None,
            parent,
            extensions: BTreeMap::from([(
                WORKFLOW_EXTENSION.to_string(),
                serde_json::to_value(metadata).unwrap(),
            )]),
        }
    }

    #[test]
    fn projects_and_selects_a_25k_container_and_task_deep_campaign_without_recursion() {
        // Arrange
        const DEPTH: usize = 25_000;
        let mut active = Vec::with_capacity(DEPTH * 2);
        for index in 0..DEPTH {
            let container_id = format!("container-{index:05}");
            let parent = (index > 0).then(|| format!("container-{:05}", index - 1));
            active.push(workflow_item(
                container_id.clone(),
                parent,
                NodeKind::Container,
            ));
            active.push(workflow_item(
                format!("task-{index:05}"),
                Some(container_id),
                NodeKind::Task,
            ));
        }

        // Act
        let projection = project_campaign(&active, &[], "container-00000").unwrap();
        let selection = select_task(&projection, "container-00000").unwrap();

        // Assert
        assert_eq!(projection.items.len(), DEPTH * 2);
        assert!(matches!(selection, SelectedTask::Claim(_)));
    }
}
