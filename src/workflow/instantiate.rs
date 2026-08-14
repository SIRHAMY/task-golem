use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::template::{
    ContextMode, NodeKind, PluginDefinition, WorkflowContext, WorkflowDefinition, WorkflowNode,
};
use crate::errors::TgError;
use crate::model::id;
use crate::model::item::Item;
use crate::model::status::Status;
use crate::store::Store;
use crate::store::config::Config;

const WORKFLOW_EXTENSION: &str = "x-workflow";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPlugin {
    pub version: u32,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMetadata {
    pub version: u32,
    pub instance: String,
    pub node: String,
    pub kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<WorkflowPlugin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<WorkflowContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    pub instance_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowInstance {
    pub campaign_id: String,
    pub instance: String,
    pub instance_digest: String,
    pub nodes: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct DigestMaterial<'a> {
    version: u32,
    name: &'a str,
    inputs: &'a BTreeMap<String, String>,
    plugins: BTreeMap<&'a str, DigestPlugin<'a>>,
    nodes: &'a [WorkflowNode],
}

#[derive(Serialize)]
struct DigestPlugin<'a> {
    version: u32,
    argv: &'a [String],
}

struct PersistedWorkflowMetadata {
    immutable: WorkflowMetadata,
    session_ref: Option<String>,
}

pub fn instantiate_workflow(
    store: &Store,
    config: &Config,
    definition: &WorkflowDefinition,
    instance: &str,
) -> Result<WorkflowInstance, TgError> {
    if instance.trim().is_empty() {
        return Err(invalid("workflow instance cannot be empty"));
    }
    let instance_digest = definition_digest(definition)?;

    store.with_lock(|store| {
        let mut active = store.load_active()?;
        let archive = store.load_all_archive_strict()?;
        let mut known_ids = collect_unique_item_ids(&active, &archive)?;
        if let Some(existing) =
            find_existing_instance(&active, &archive, definition, instance, &instance_digest)?
        {
            return Ok(existing);
        }

        let mut node_ids = BTreeMap::new();
        for node in &definition.nodes {
            let item_id =
                id::generate_id_with_prefix(&known_ids, &config.id_prefix, config.id_len)?;
            known_ids.insert(item_id.clone());
            if node_ids.insert(node.id.clone(), item_id).is_some() {
                return Err(invalid(format!("duplicate workflow node id '{}'", node.id)));
            }
        }

        let created_at = Utc::now();
        let created = build_items(
            definition,
            instance,
            &instance_digest,
            &node_ids,
            created_at,
        )?;
        let campaign_id = campaign_id(definition, &node_ids)?;
        active.extend(created);
        store.save_active(&active)?;

        Ok(WorkflowInstance {
            campaign_id,
            instance: instance.to_string(),
            instance_digest,
            nodes: node_ids,
        })
    })
}

fn definition_digest(definition: &WorkflowDefinition) -> Result<String, TgError> {
    let plugins = definition
        .plugins
        .iter()
        .map(|(alias, plugin)| {
            (
                alias.as_str(),
                DigestPlugin {
                    version: plugin.version,
                    argv: &plugin.argv,
                },
            )
        })
        .collect();
    let material = DigestMaterial {
        version: definition.version,
        name: &definition.name,
        inputs: &definition.inputs,
        plugins,
        nodes: &definition.nodes,
    };
    let canonical = serde_json::to_vec(&material)
        .map_err(|error| invalid(format!("cannot canonicalize workflow definition: {error}")))?;
    Ok(format!("sha256:{:x}", Sha256::digest(canonical)))
}

fn build_items(
    definition: &WorkflowDefinition,
    instance: &str,
    instance_digest: &str,
    node_ids: &BTreeMap<String, String>,
    created_at: DateTime<Utc>,
) -> Result<Vec<Item>, TgError> {
    definition
        .nodes
        .iter()
        .map(|node| {
            let item_id = mapped_id(node_ids, &node.id)?;
            let parent = node
                .parent
                .as_deref()
                .map(|parent| mapped_id(node_ids, parent))
                .transpose()?;
            let dependencies = node
                .depends_on
                .iter()
                .map(|dependency| mapped_id(node_ids, dependency))
                .collect::<Result<Vec<_>, _>>()?;
            let metadata = workflow_metadata(definition, node, instance, instance_digest)?;
            let mut extensions = BTreeMap::new();
            extensions.insert(
                WORKFLOW_EXTENSION.to_string(),
                serde_json::to_value(metadata).map_err(|error| {
                    invalid(format!("cannot serialize workflow metadata: {error}"))
                })?,
            );
            let item = Item {
                id: item_id,
                title: node.title.clone(),
                status: Status::Todo,
                priority: 0,
                description: node.description.clone(),
                tags: vec![],
                dependencies,
                created_at,
                updated_at: created_at,
                blocked_reason: None,
                blocked_from_status: None,
                claimed_by: None,
                claimed_at: None,
                parent,
                extensions,
            };
            Item::validate_title(&item.title)?;
            item.validate_extensions()?;
            Ok(item)
        })
        .collect()
}

fn workflow_metadata(
    definition: &WorkflowDefinition,
    node: &WorkflowNode,
    instance: &str,
    instance_digest: &str,
) -> Result<WorkflowMetadata, TgError> {
    let (plugin, context, input, verify) = match node.kind {
        NodeKind::Container => (None, None, None, None),
        NodeKind::Task => {
            let plugin_alias = node
                .plugin
                .as_deref()
                .ok_or_else(|| invalid(format!("task '{}' must declare plugin", node.id)))?;
            let plugin = definition.plugins.get(plugin_alias).ok_or_else(|| {
                invalid(format!(
                    "task '{}' references unknown plugin '{plugin_alias}'",
                    node.id
                ))
            })?;
            let context = node
                .context
                .clone()
                .ok_or_else(|| invalid(format!("task '{}' must declare context", node.id)))?;
            (
                Some(project_plugin(plugin)),
                Some(context),
                Some(node.input.clone()),
                node.verify.clone(),
            )
        }
    };

    Ok(WorkflowMetadata {
        version: definition.version,
        instance: instance.to_string(),
        node: node.id.clone(),
        kind: node.kind,
        plugin,
        context,
        input,
        verify,
        instance_digest: instance_digest.to_string(),
    })
}

fn project_plugin(plugin: &PluginDefinition) -> WorkflowPlugin {
    WorkflowPlugin {
        version: plugin.version,
        argv: plugin.argv.clone(),
    }
}

fn campaign_id(
    definition: &WorkflowDefinition,
    node_ids: &BTreeMap<String, String>,
) -> Result<String, TgError> {
    let root = definition
        .nodes
        .iter()
        .find(|node| node.parent.is_none() && node.kind == NodeKind::Container)
        .ok_or_else(|| invalid("workflow definition has no root container"))?;
    mapped_id(node_ids, &root.id)
}

fn mapped_id(node_ids: &BTreeMap<String, String>, node: &str) -> Result<String, TgError> {
    node_ids
        .get(node)
        .cloned()
        .ok_or_else(|| invalid(format!("workflow references missing node '{node}'")))
}

fn collect_unique_item_ids(active: &[Item], archive: &[Item]) -> Result<HashSet<String>, TgError> {
    let mut item_ids = HashSet::new();
    for item in active.iter().chain(archive.iter()) {
        if !item_ids.insert(item.id.clone()) {
            return Err(TgError::StorageCorruption(format!(
                "duplicate item ID '{}' in active/archive records",
                item.id
            )));
        }
    }
    Ok(item_ids)
}

fn find_existing_instance(
    active: &[Item],
    archive: &[Item],
    definition: &WorkflowDefinition,
    instance: &str,
    expected_digest: &str,
) -> Result<Option<WorkflowInstance>, TgError> {
    let workflow_items = active
        .iter()
        .chain(archive.iter())
        .filter_map(|item| {
            item.extensions
                .get(WORKFLOW_EXTENSION)
                .map(|metadata| (item, metadata))
        })
        .map(|(item, metadata)| {
            let mut immutable_metadata = metadata.clone();
            let session_ref = immutable_metadata
                .as_object_mut()
                .and_then(|fields| fields.remove("session_ref"));
            serde_json::from_value::<WorkflowMetadata>(immutable_metadata)
                .map_err(|error| {
                    TgError::StorageCorruption(format!(
                        "invalid x-workflow metadata for item '{}': {error}",
                        item.id
                    ))
                })
                .and_then(|immutable| {
                    let session_ref = match session_ref {
                        None => None,
                        Some(serde_json::Value::String(session_ref))
                            if !session_ref.trim().is_empty() =>
                        {
                            Some(session_ref)
                        }
                        Some(_) => {
                            return Err(TgError::StorageCorruption(format!(
                                "invalid x-workflow metadata for item '{}': session_ref must be a non-empty string",
                                item.id
                            )));
                        }
                    };
                    Ok((
                        item,
                        PersistedWorkflowMetadata {
                            immutable,
                            session_ref,
                        },
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let matching: Vec<(&Item, &PersistedWorkflowMetadata)> = workflow_items
        .iter()
        .filter(|(_, metadata)| metadata.immutable.instance == instance)
        .map(|(item, metadata)| (*item, metadata))
        .collect();
    if matching.is_empty() {
        return Ok(None);
    }
    if matching
        .iter()
        .any(|(_, metadata)| metadata.immutable.instance_digest != expected_digest)
    {
        return Err(invalid(format!(
            "workflow instance '{instance}' already exists with a different digest"
        )));
    }

    let mut persisted_by_node = BTreeMap::new();
    let mut persisted_item_ids = HashSet::new();
    let mut nodes = BTreeMap::new();
    for &(item, metadata) in &matching {
        let metadata = &metadata.immutable;
        if persisted_by_node
            .insert(metadata.node.as_str(), (item, metadata))
            .is_some()
        {
            return Err(TgError::StorageCorruption(format!(
                "workflow instance '{instance}' has duplicate node '{}'",
                metadata.node
            )));
        }
        if !persisted_item_ids.insert(item.id.as_str()) {
            return Err(corrupt_graph(
                instance,
                &format!("multiple nodes map to item ID '{}'", item.id),
            ));
        }
        nodes.insert(metadata.node.clone(), item.id.clone());
    }

    let expected_nodes: BTreeSet<&str> = definition
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    let persisted_nodes: BTreeSet<&str> = nodes.keys().map(String::as_str).collect();
    if persisted_nodes != expected_nodes {
        return Err(corrupt_graph(
            instance,
            "node set differs from the definition",
        ));
    }

    let shared_context_owners: BTreeSet<&str> = definition
        .nodes
        .iter()
        .filter_map(|node| {
            node.context.as_ref().and_then(|context| {
                (context.mode == ContextMode::Shared)
                    .then_some(context.key.as_deref())
                    .flatten()
            })
        })
        .collect();
    for &(item, metadata) in &matching {
        if metadata.session_ref.is_some()
            && (metadata.immutable.kind != NodeKind::Container
                || !shared_context_owners.contains(metadata.immutable.node.as_str()))
        {
            return Err(corrupt_graph(
                instance,
                &format!(
                    "item '{}' stores session_ref outside a shared-context owner container",
                    item.id
                ),
            ));
        }
    }

    for node in &definition.nodes {
        let (item, metadata) = persisted_by_node
            .get(node.id.as_str())
            .copied()
            .ok_or_else(|| corrupt_graph(instance, "node set differs from the definition"))?;
        if metadata.version != definition.version
            || metadata.instance != instance
            || metadata.kind != node.kind
            || metadata.instance_digest != expected_digest
        {
            return Err(corrupt_graph(
                instance,
                &format!("node '{}' identity differs from the definition", node.id),
            ));
        }
        if item.title != node.title || item.description != node.description {
            return Err(corrupt_graph(
                instance,
                &format!(
                    "node '{}' item content differs from the definition",
                    node.id
                ),
            ));
        }

        let expected_metadata = workflow_metadata(definition, node, instance, expected_digest)?;
        if metadata.plugin != expected_metadata.plugin
            || metadata.context != expected_metadata.context
            || metadata.input != expected_metadata.input
            || metadata.verify != expected_metadata.verify
        {
            return Err(corrupt_graph(
                instance,
                &format!(
                    "node '{}' execution projection differs from the definition",
                    node.id
                ),
            ));
        }

        let expected_parent = node
            .parent
            .as_deref()
            .map(|parent| mapped_id(&nodes, parent))
            .transpose()?;
        if item.parent != expected_parent {
            return Err(corrupt_graph(
                instance,
                &format!("node '{}' parent differs from the definition", node.id),
            ));
        }

        let expected_dependencies = node
            .depends_on
            .iter()
            .map(|dependency| mapped_id(&nodes, dependency))
            .collect::<Result<Vec<_>, _>>()?;
        if item.dependencies != expected_dependencies {
            return Err(corrupt_graph(
                instance,
                &format!("node '{}' dependencies differ from the definition", node.id),
            ));
        }
    }

    let campaign_id = campaign_id(definition, &nodes)?;

    Ok(Some(WorkflowInstance {
        campaign_id,
        instance: instance.to_string(),
        instance_digest: expected_digest.to_string(),
        nodes,
    }))
}

fn invalid(message: impl Into<String>) -> TgError {
    TgError::InvalidInput(message.into())
}

fn corrupt_graph(instance: &str, detail: &str) -> TgError {
    TgError::StorageCorruption(format!(
        "workflow instance '{instance}' does not match its expected graph: {detail}"
    ))
}
