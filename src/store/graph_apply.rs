use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::Utc;

use super::{Store, jsonl};
use crate::errors::TgError;
use crate::model::deps;
use crate::model::graph::{
    GraphApplyCategory, GraphApplyDiagnostic, GraphApplyDiagnosticCode, GraphApplyError,
    GraphApplyRequest, GraphApplyResult, GraphRef,
};
use crate::model::id;
use crate::model::item::Item;
use crate::model::status::Status;

struct GraphSnapshot {
    active: Vec<Item>,
    active_ids: HashSet<String>,
    archive_by_id: HashMap<String, Status>,
}

impl Store {
    /// Create every requested item under one lock and one atomic task-store write.
    pub fn apply_graph(&self, request: GraphApplyRequest) -> Result<GraphApplyResult, TgError> {
        self.with_lock(|store| {
            apply_graph_locked(store, request, |path, items| {
                jsonl::write_atomic(path, items)
            })
            .map_err(TgError::from)
        })
        .map_err(|error| match error {
            TgError::GraphApply(_) => error,
            other => persistence_error("store.lock", other).into(),
        })
    }
}

fn apply_graph_locked(
    store: &Store,
    request: GraphApplyRequest,
    write_active: impl FnOnce(&Path, &[Item]) -> Result<(), TgError>,
) -> Result<GraphApplyResult, GraphApplyError> {
    let snapshot = load_snapshot(store)?;
    validate_request(&request)?;

    let mut known_ids = snapshot.active_ids.clone();
    known_ids.extend(snapshot.archive_by_id.keys().cloned());
    let mut mapping = BTreeMap::new();
    for (index, item) in request.items.iter().enumerate() {
        let generated_id = id::generate_id(&known_ids)
            .map_err(|error| persistence_error(&format!("items[{index}].id"), error))?;
        known_ids.insert(generated_id.clone());
        mapping.insert(item.reference.clone(), generated_id);
    }

    let created_at = Utc::now();
    let new_items = resolve_items(&request, &mapping, &snapshot, created_at)?;
    let mut proposed_active = snapshot.active.clone();
    proposed_active.extend(new_items.clone());
    validate_proposed_graph(&request, &mapping, &new_items, &proposed_active)?;

    write_active(&store.tasks_path(), &proposed_active)
        .map_err(|error| persistence_error("tasks.jsonl", error))?;

    Ok(GraphApplyResult::created(mapping))
}

fn load_snapshot(store: &Store) -> Result<GraphSnapshot, GraphApplyError> {
    let active = store
        .load_active()
        .map_err(|error| snapshot_error("tasks.jsonl", error))?;
    let archive = jsonl::read_archive_strict(&store.archive_path())
        .map_err(|error| snapshot_error("archive.jsonl", error))?;

    let (active_ids, archive_by_id) = validate_snapshot(&active, &archive)?;
    Ok(GraphSnapshot {
        active,
        active_ids,
        archive_by_id,
    })
}

fn validate_snapshot(
    active: &[Item],
    archive: &[Item],
) -> Result<(HashSet<String>, HashMap<String, Status>), GraphApplyError> {
    let mut diagnostics = Vec::new();
    let mut first_identity_path = HashMap::new();
    let mut active_ids = HashSet::new();
    let mut archive_by_id = HashMap::new();

    for (index, item) in active.iter().enumerate() {
        let path = format!("active[{index}]");
        validate_durable_item(item, &path, &mut diagnostics);
        record_durable_identity(
            &item.id,
            format!("{path}.id"),
            &mut first_identity_path,
            &mut diagnostics,
        );
        active_ids.insert(item.id.clone());
    }
    for (index, item) in archive.iter().enumerate() {
        let path = format!("archive[{index}]");
        validate_durable_item(item, &path, &mut diagnostics);
        record_durable_identity(
            &item.id,
            format!("{path}.id"),
            &mut first_identity_path,
            &mut diagnostics,
        );
        archive_by_id.insert(item.id.clone(), item.status);
    }

    for (index, item) in active.iter().enumerate() {
        if let Some(parent_id) = &item.parent {
            let path = format!("active[{index}].parent");
            validate_durable_edge_id(parent_id, &path, &mut diagnostics);
            if parent_id == &item.id {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::SelfReference, path)
                        .with_reference(GraphRef::Existing(parent_id.clone())),
                );
            } else if !active_ids.contains(parent_id) {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::MissingReference, path)
                        .with_reference(GraphRef::Existing(parent_id.clone())),
                );
            }
        }

        let mut first_dependency_path: HashMap<String, String> = HashMap::new();
        for (dependency_index, dependency_id) in item.dependencies.iter().enumerate() {
            let path = format!("active[{index}].dependencies[{dependency_index}]");
            validate_durable_edge_id(dependency_id, &path, &mut diagnostics);
            if dependency_id == &item.id {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::SelfReference, &path)
                        .with_reference(GraphRef::Existing(dependency_id.clone())),
                );
            }
            if !active_ids.contains(dependency_id) && !archive_by_id.contains_key(dependency_id) {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::MissingReference, &path)
                        .with_reference(GraphRef::Existing(dependency_id.clone())),
                );
            }
            if let Some(first_path) = first_dependency_path.get(dependency_id) {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::DuplicateDependency, path)
                        .with_reference(GraphRef::Existing(dependency_id.clone()))
                        .with_detail("first_path", first_path.clone()),
                );
            } else {
                first_dependency_path.insert(dependency_id.clone(), path);
            }
        }
    }

    append_durable_cycle_diagnostics(
        deps::detect_all_parent_cycles(active),
        GraphApplyDiagnosticCode::ParentCycle,
        &mut diagnostics,
    );
    append_durable_cycle_diagnostics(
        deps::detect_all_cycles(active),
        GraphApplyDiagnosticCode::DependencyCycle,
        &mut diagnostics,
    );

    if diagnostics.is_empty() {
        Ok((active_ids, archive_by_id))
    } else {
        Err(GraphApplyError::new(
            GraphApplyCategory::StorageCorruption,
            diagnostics,
        ))
    }
}

fn validate_durable_item(item: &Item, path: &str, diagnostics: &mut Vec<GraphApplyDiagnostic>) {
    if id::validate_id(&item.id).is_err() {
        diagnostics.push(
            GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidId, format!("{path}.id"))
                .with_reference(GraphRef::Existing(item.id.clone())),
        );
    }
    if let Err(error) = Item::validate_title(&item.title) {
        diagnostics.push(
            GraphApplyDiagnostic::new(
                GraphApplyDiagnosticCode::InvalidItem,
                format!("{path}.title"),
            )
            .with_detail("message", error.to_string()),
        );
    }
    if let Err(error) = item.validate_extensions() {
        diagnostics.push(
            GraphApplyDiagnostic::new(
                GraphApplyDiagnosticCode::InvalidItem,
                format!("{path}.extensions"),
            )
            .with_detail("message", error.to_string()),
        );
    }
}

fn record_durable_identity<'a>(
    item_id: &'a str,
    path: String,
    first_identity_path: &mut HashMap<&'a str, String>,
    diagnostics: &mut Vec<GraphApplyDiagnostic>,
) {
    if let Some(first_path) = first_identity_path.get(item_id) {
        diagnostics.push(
            GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::DuplicateDurableId, path)
                .with_reference(GraphRef::Existing(item_id.to_string()))
                .with_detail("first_path", first_path.clone()),
        );
    } else {
        first_identity_path.insert(item_id, path);
    }
}

fn validate_durable_edge_id(
    item_id: &str,
    path: &str,
    diagnostics: &mut Vec<GraphApplyDiagnostic>,
) {
    if id::validate_id(item_id).is_err() {
        diagnostics.push(
            GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidId, path)
                .with_reference(GraphRef::Existing(item_id.to_string())),
        );
    }
}

fn append_durable_cycle_diagnostics(
    cycles: Vec<Vec<String>>,
    code: GraphApplyDiagnosticCode,
    diagnostics: &mut Vec<GraphApplyDiagnostic>,
) {
    for cycle in normalize_cycles(cycles, &HashMap::new()) {
        diagnostics.push(GraphApplyDiagnostic::new(code, "active").with_detail("cycle", cycle));
    }
}

fn validate_request(request: &GraphApplyRequest) -> Result<(), GraphApplyError> {
    let mut diagnostics = Vec::new();
    if request.items.is_empty() {
        diagnostics.push(GraphApplyDiagnostic::new(
            GraphApplyDiagnosticCode::EmptyGraph,
            "items",
        ));
    }

    let mut first_reference_path: HashMap<&str, String> = HashMap::new();
    for (index, item) in request.items.iter().enumerate() {
        let reference_path = format!("items[{index}].ref");
        if item.reference.trim().is_empty()
            || item.reference.contains('\n')
            || item.reference.contains('\r')
        {
            diagnostics.push(
                GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidItem, &reference_path)
                    .with_reference(GraphRef::Local(item.reference.clone()))
                    .with_detail("message", "ref must be a non-empty single line"),
            );
        }
        if let Some(first_path) = first_reference_path.get(item.reference.as_str()) {
            diagnostics.push(
                GraphApplyDiagnostic::new(
                    GraphApplyDiagnosticCode::DuplicateReference,
                    reference_path,
                )
                .with_reference(GraphRef::Local(item.reference.clone()))
                .with_detail("first_path", first_path.clone()),
            );
        } else {
            first_reference_path.insert(item.reference.as_str(), reference_path);
        }

        if let Err(error) = Item::validate_title(&item.title) {
            diagnostics.push(
                GraphApplyDiagnostic::new(
                    GraphApplyDiagnosticCode::InvalidItem,
                    format!("items[{index}].title"),
                )
                .with_reference(GraphRef::Local(item.reference.clone()))
                .with_detail("message", error.to_string()),
            );
        }
        for extension_key in item.extensions.keys() {
            if !extension_key.starts_with("x-") {
                diagnostics.push(
                    GraphApplyDiagnostic::new(
                        GraphApplyDiagnosticCode::InvalidItem,
                        format!("items[{index}].extensions.{extension_key}"),
                    )
                    .with_reference(GraphRef::Local(item.reference.clone()))
                    .with_detail("message", "extension keys must start with 'x-'"),
                );
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(GraphApplyError::new(
            GraphApplyCategory::InvalidRequest,
            diagnostics,
        ))
    }
}

fn resolve_items(
    request: &GraphApplyRequest,
    mapping: &BTreeMap<String, String>,
    snapshot: &GraphSnapshot,
    created_at: chrono::DateTime<Utc>,
) -> Result<Vec<Item>, GraphApplyError> {
    let mut diagnostics = Vec::new();
    let mut resolved_items = Vec::with_capacity(request.items.len());

    for (index, requested_item) in request.items.iter().enumerate() {
        let item_id = mapping
            .get(&requested_item.reference)
            .expect("validated request references must have generated IDs");
        let parent = requested_item.parent.as_ref().and_then(|target| {
            resolve_target(
                target,
                EdgeKind::Parent,
                &format!("items[{index}].parent"),
                mapping,
                snapshot,
                &mut diagnostics,
            )
        });
        if parent.as_deref() == Some(item_id) {
            diagnostics.push(
                GraphApplyDiagnostic::new(
                    GraphApplyDiagnosticCode::SelfReference,
                    format!("items[{index}].parent"),
                )
                .with_reference(
                    requested_item
                        .parent
                        .clone()
                        .expect("resolved parent has a source reference"),
                ),
            );
        }

        let mut dependencies = Vec::with_capacity(requested_item.dependencies.len());
        let mut first_dependency_path: HashMap<String, String> = HashMap::new();
        for (dependency_index, target) in requested_item.dependencies.iter().enumerate() {
            let path = format!("items[{index}].dependencies[{dependency_index}]");
            let Some(dependency_id) = resolve_target(
                target,
                EdgeKind::Dependency,
                &path,
                mapping,
                snapshot,
                &mut diagnostics,
            ) else {
                continue;
            };
            if &dependency_id == item_id {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::SelfReference, &path)
                        .with_reference(target.clone()),
                );
            }
            if let Some(first_path) = first_dependency_path.get(&dependency_id) {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::DuplicateDependency, path)
                        .with_reference(target.clone())
                        .with_detail("first_path", first_path.clone()),
                );
            } else {
                first_dependency_path.insert(dependency_id.clone(), path);
            }
            dependencies.push(dependency_id);
        }

        resolved_items.push(Item {
            id: item_id.clone(),
            title: requested_item.title.clone(),
            status: Status::Todo,
            priority: requested_item.priority,
            description: requested_item.description.clone(),
            tags: deduplicate(&requested_item.tags),
            dependencies,
            created_at,
            updated_at: created_at,
            blocked_reason: None,
            blocked_from_status: None,
            claimed_by: None,
            claimed_at: None,
            parent,
            extensions: requested_item.extensions.clone(),
        });
    }

    if diagnostics.is_empty() {
        Ok(resolved_items)
    } else {
        Err(GraphApplyError::new(
            GraphApplyCategory::InvalidRequest,
            diagnostics,
        ))
    }
}

fn deduplicate(values: &[String]) -> Vec<String> {
    let mut unique = Vec::with_capacity(values.len());
    for value in values {
        if !unique.contains(value) {
            unique.push(value.clone());
        }
    }
    unique
}

#[derive(Clone, Copy)]
enum EdgeKind {
    Parent,
    Dependency,
}

fn resolve_target(
    target: &GraphRef,
    edge_kind: EdgeKind,
    path: &str,
    mapping: &BTreeMap<String, String>,
    snapshot: &GraphSnapshot,
    diagnostics: &mut Vec<GraphApplyDiagnostic>,
) -> Option<String> {
    match target {
        GraphRef::Local(reference) => mapping.get(reference).cloned().or_else(|| {
            diagnostics.push(
                GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::MissingReference, path)
                    .with_reference(target.clone()),
            );
            None
        }),
        GraphRef::Existing(item_id) => {
            if id::validate_id(item_id).is_err() {
                diagnostics.push(
                    GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidId, path)
                        .with_reference(target.clone()),
                );
                return None;
            }
            if snapshot.active_ids.contains(item_id) {
                return Some(item_id.clone());
            }
            if matches!(edge_kind, EdgeKind::Dependency)
                && snapshot.archive_by_id.get(item_id) == Some(&Status::Done)
            {
                return Some(item_id.clone());
            }

            let reason = match (edge_kind, snapshot.archive_by_id.get(item_id)) {
                (EdgeKind::Parent, Some(_)) => "parent_target_is_archived",
                (EdgeKind::Dependency, Some(_)) => "archived_dependency_has_no_completion_evidence",
                (_, None) => "target_not_found",
            };
            diagnostics.push(
                GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::MissingReference, path)
                    .with_reference(target.clone())
                    .with_detail("reason", reason),
            );
            None
        }
    }
}

fn validate_proposed_graph(
    request: &GraphApplyRequest,
    mapping: &BTreeMap<String, String>,
    new_items: &[Item],
    proposed_active: &[Item],
) -> Result<(), GraphApplyError> {
    let local_by_id: HashMap<String, String> = mapping
        .iter()
        .map(|(reference, item_id)| (item_id.clone(), reference.clone()))
        .collect();
    let mut diagnostics = Vec::new();

    append_request_cycle_diagnostics(
        deps::detect_all_parent_cycles(proposed_active),
        GraphApplyDiagnosticCode::ParentCycle,
        request,
        new_items,
        &local_by_id,
        &mut diagnostics,
    );
    append_request_cycle_diagnostics(
        deps::detect_all_cycles(proposed_active),
        GraphApplyDiagnosticCode::DependencyCycle,
        request,
        new_items,
        &local_by_id,
        &mut diagnostics,
    );

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(GraphApplyError::new(
            GraphApplyCategory::InvalidGraph,
            diagnostics,
        ))
    }
}

fn append_request_cycle_diagnostics(
    cycles: Vec<Vec<String>>,
    code: GraphApplyDiagnosticCode,
    request: &GraphApplyRequest,
    new_items: &[Item],
    local_by_id: &HashMap<String, String>,
    diagnostics: &mut Vec<GraphApplyDiagnostic>,
) {
    for cycle in normalize_cycles(cycles, local_by_id) {
        let cycle_ids: HashSet<&str> = cycle
            .iter()
            .filter_map(|reference| match reference {
                GraphRef::Local(local) => local_by_id.iter().find_map(|(item_id, candidate)| {
                    (candidate == local).then_some(item_id.as_str())
                }),
                GraphRef::Existing(item_id) => Some(item_id.as_str()),
            })
            .collect();
        let path = cycle_source_path(code, request, new_items, &cycle_ids);
        diagnostics.push(GraphApplyDiagnostic::new(code, path).with_detail("cycle", cycle));
    }
}

fn cycle_source_path(
    code: GraphApplyDiagnosticCode,
    request: &GraphApplyRequest,
    new_items: &[Item],
    cycle_ids: &HashSet<&str>,
) -> String {
    let mut paths = Vec::new();
    for (item_index, item) in new_items.iter().enumerate() {
        if !cycle_ids.contains(item.id.as_str()) {
            continue;
        }
        match code {
            GraphApplyDiagnosticCode::ParentCycle => {
                if item
                    .parent
                    .as_deref()
                    .is_some_and(|parent| cycle_ids.contains(parent))
                {
                    paths.push(format!("items[{item_index}].parent"));
                }
            }
            GraphApplyDiagnosticCode::DependencyCycle => {
                for (dependency_index, dependency_id) in item.dependencies.iter().enumerate() {
                    if cycle_ids.contains(dependency_id.as_str()) {
                        paths.push(format!(
                            "items[{item_index}].dependencies[{dependency_index}]"
                        ));
                    }
                }
            }
            _ => unreachable!("only graph cycle codes have cycle source paths"),
        }
    }

    paths.sort();
    paths
        .into_iter()
        .next()
        .unwrap_or_else(|| format!("items[{}]", request.items.len()))
}

fn normalize_cycles(
    cycles: Vec<Vec<String>>,
    local_by_id: &HashMap<String, String>,
) -> Vec<Vec<GraphRef>> {
    let mut normalized = cycles
        .into_iter()
        .filter(|cycle| !cycle.is_empty())
        .map(|cycle| {
            let translated = cycle
                .into_iter()
                .map(|item_id| match local_by_id.get(&item_id) {
                    Some(reference) => GraphRef::Local(reference.clone()),
                    None => GraphRef::Existing(item_id),
                })
                .collect::<Vec<_>>();
            let smallest_index = translated
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| left.cmp(right))
                .map(|(index, _)| index)
                .expect("non-empty cycle has a smallest member");
            translated[smallest_index..]
                .iter()
                .chain(&translated[..smallest_index])
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn snapshot_error(path: &str, error: TgError) -> GraphApplyError {
    match error {
        TgError::IoError(_) | TgError::LockTimeout(_) => persistence_error(path, error),
        other => GraphApplyError::new(
            GraphApplyCategory::StorageCorruption,
            vec![
                GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidItem, path)
                    .with_detail("message", other.to_string()),
            ],
        ),
    }
}

fn persistence_error(path: &str, error: TgError) -> GraphApplyError {
    GraphApplyError::new(
        GraphApplyCategory::PersistenceFailure,
        vec![
            GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::PersistenceFailure, path)
                .with_detail("message", error.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GraphApplyRequest {
        GraphApplyRequest {
            items: vec![crate::model::graph::GraphApplyItem {
                reference: "root".to_string(),
                title: "Root".to_string(),
                description: None,
                priority: 0,
                tags: vec![],
                parent: None,
                dependencies: vec![],
                extensions: BTreeMap::new(),
            }],
        }
    }

    #[test]
    fn persistence_failure_returns_typed_error_without_mutating_durable_state() {
        // Arrange
        let temporary_directory = tempfile::tempdir().unwrap();
        let store = Store::new(temporary_directory.path().to_path_buf());
        jsonl::write_empty(&store.tasks_path()).unwrap();
        jsonl::write_empty(&store.archive_path()).unwrap();
        let tasks_before = std::fs::read(store.tasks_path()).unwrap();
        let archive_before = std::fs::read(store.archive_path()).unwrap();

        // Act
        let error = apply_graph_locked(&store, request(), |_path, _items| {
            Err(TgError::IoError(std::io::Error::other("injected failure")))
        })
        .unwrap_err();

        // Assert
        assert_eq!(error.category, GraphApplyCategory::PersistenceFailure);
        assert_eq!(
            error.diagnostics[0].code,
            GraphApplyDiagnosticCode::PersistenceFailure
        );
        assert_eq!(std::fs::read(store.tasks_path()).unwrap(), tasks_before);
        assert_eq!(std::fs::read(store.archive_path()).unwrap(), archive_before);
        assert!(!store.events_path().exists());
        assert!(!store.events_archive_path().exists());
    }
}
