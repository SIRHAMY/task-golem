use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::errors::TgError;
use crate::model::item::Item;

const WORKFLOW_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Container,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextMode {
    Fresh,
    Shared,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowContext {
    pub mode: ContextMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: NodeKind,
    pub parent: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub plugin: Option<String>,
    pub context: Option<WorkflowContext>,
    pub input: BTreeMap<String, JsonValue>,
    pub verify: Option<Vec<String>>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDefinition {
    pub version: u32,
    pub argv: Vec<String>,
    pub definition_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowDefinition {
    pub version: u32,
    pub name: String,
    pub template_path: PathBuf,
    pub inputs: BTreeMap<String, String>,
    pub plugins: BTreeMap<String, PluginDefinition>,
    pub nodes: Vec<WorkflowNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRequest {
    pub version: u32,
    pub campaign_id: String,
    pub task_id: String,
    pub title: String,
    pub description: Option<String>,
    pub workspace: PathBuf,
    pub input: BTreeMap<String, JsonValue>,
    pub context: PluginRequestContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRequestContext {
    pub mode: ContextMode,
    pub key: Option<String>,
    pub session_ref: Option<String>,
    pub resume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginResultStatus {
    Complete,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginResult {
    pub version: u32,
    pub status: PluginResultStatus,
    pub summary: String,
    pub session_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateFile {
    version: u32,
    name: String,
    plugins: BTreeMap<String, String>,
    nodes: Vec<TemplateNode>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateNode {
    id: String,
    kind: NodeKind,
    parent: Option<String>,
    title: String,
    description: Option<String>,
    plugin: Option<String>,
    context: Option<String>,
    input: Option<BTreeMap<String, JsonValue>>,
    verify: Option<Vec<String>>,
    depends_on: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginFile {
    version: u32,
    argv: Vec<String>,
}

pub fn load_workflow_definition(
    project_dir: &Path,
    template_path: &Path,
    input_args: &[String],
) -> Result<WorkflowDefinition, TgError> {
    let workspace_root = project_dir
        .parent()
        .ok_or_else(|| invalid("task-golem project directory has no workspace parent"))?;
    let workspace_root = fs::canonicalize(workspace_root).map_err(TgError::IoError)?;
    let template_path = canonical_definition_path(&workspace_root, template_path, "template")?;
    let template_source = fs::read_to_string(&template_path).map_err(TgError::IoError)?;
    let (resolved_source, inputs) = substitute_inputs(&template_source, input_args)?;
    let template: TemplateFile = serde_yaml::from_value(resolved_source)
        .map_err(|error| invalid(format!("invalid workflow template: {error}")))?;

    validate_template_header(&template)?;
    let plugins = load_plugins(&workspace_root, &template.plugins)?;
    let nodes = validate_nodes(template.nodes, &plugins)?;

    Ok(WorkflowDefinition {
        version: template.version,
        name: template.name,
        template_path,
        inputs,
        plugins,
        nodes,
    })
}

pub fn parse_plugin_request(json: &str) -> Result<PluginRequest, TgError> {
    let request: PluginRequest = serde_json::from_str(json)
        .map_err(|error| invalid(format!("invalid plugin request: {error}")))?;
    validate_version(request.version, "plugin request")?;
    validate_nonempty(&request.campaign_id, "plugin request campaign_id")?;
    validate_nonempty(&request.task_id, "plugin request task_id")?;
    validate_nonempty(&request.title, "plugin request title")?;
    if !request.workspace.is_absolute() {
        return Err(invalid("plugin request workspace must be absolute"));
    }
    validate_request_context(&request.context)?;
    Ok(request)
}

pub fn parse_plugin_result(json: &str) -> Result<PluginResult, TgError> {
    let result: PluginResult = serde_json::from_str(json)
        .map_err(|error| invalid(format!("invalid plugin result: {error}")))?;
    validate_version(result.version, "plugin result")?;
    validate_nonempty(&result.summary, "plugin result summary")?;
    if let Some(session_ref) = &result.session_ref {
        validate_nonempty(session_ref, "plugin result session_ref")?;
    }
    Ok(result)
}

fn canonical_definition_path(
    workspace_root: &Path,
    definition_path: &Path,
    kind: &str,
) -> Result<PathBuf, TgError> {
    let candidate = if definition_path.is_absolute() {
        definition_path.to_path_buf()
    } else {
        workspace_root.join(definition_path)
    };
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        invalid(format!(
            "cannot resolve workflow {kind} '{}': {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(workspace_root) {
        return Err(invalid(format!(
            "workflow {kind} '{}' is outside workspace root '{}'",
            canonical.display(),
            workspace_root.display()
        )));
    }
    if !canonical.is_file() {
        return Err(invalid(format!(
            "workflow {kind} '{}' is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn substitute_inputs(
    source: &str,
    input_args: &[String],
) -> Result<(YamlValue, BTreeMap<String, String>), TgError> {
    let mut yaml: YamlValue = serde_yaml::from_str(source)
        .map_err(|error| invalid(format!("invalid workflow template: {error}")))?;
    let inputs = parse_inputs(input_args)?;
    let mut referenced = BTreeSet::new();
    collect_input_references(&yaml, &inputs, &mut referenced, SubstitutionLocation::Root);

    let supplied: BTreeSet<String> = inputs.keys().cloned().collect();
    let missing: Vec<&String> = referenced.difference(&supplied).collect();
    if !missing.is_empty() {
        return Err(invalid(format!(
            "missing workflow inputs: {}",
            missing
                .iter()
                .map(|key| key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let extra: Vec<&String> = supplied.difference(&referenced).collect();
    if !extra.is_empty() {
        return Err(invalid(format!(
            "unexpected workflow inputs: {}",
            extra
                .iter()
                .map(|key| key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    replace_input_references(&mut yaml, &inputs, SubstitutionLocation::Root)?;
    Ok((yaml, inputs))
}

fn parse_inputs(input_args: &[String]) -> Result<BTreeMap<String, String>, TgError> {
    input_args
        .iter()
        .try_fold(BTreeMap::new(), |mut inputs, input| {
            let (key, value) = input.split_once('=').ok_or_else(|| {
                invalid(format!(
                    "invalid workflow input '{input}': expected key=value"
                ))
            })?;
            validate_nonempty(key, "workflow input key")?;
            if inputs.insert(key.to_string(), value.to_string()).is_some() {
                return Err(invalid(format!("duplicate workflow input '{key}'")));
            }
            Ok(inputs)
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SubstitutionLocation {
    Root,
    Nodes,
    Node,
    Other,
}

fn collect_input_references(
    value: &YamlValue,
    inputs: &BTreeMap<String, String>,
    references: &mut BTreeSet<String>,
    location: SubstitutionLocation,
) {
    match value {
        YamlValue::Sequence(values) => {
            let child_location = if location == SubstitutionLocation::Nodes {
                SubstitutionLocation::Node
            } else {
                SubstitutionLocation::Other
            };
            for value in values {
                collect_input_references(value, inputs, references, child_location);
            }
        }
        YamlValue::Mapping(values) => {
            for (key, value) in values {
                collect_input_references(key, inputs, references, SubstitutionLocation::Other);
                let key = resolved_scalar(key, inputs);
                if location != SubstitutionLocation::Node || key != Some("verify") {
                    let child_location =
                        if location == SubstitutionLocation::Root && key == Some("nodes") {
                            SubstitutionLocation::Nodes
                        } else {
                            SubstitutionLocation::Other
                        };
                    collect_input_references(value, inputs, references, child_location);
                }
            }
        }
        YamlValue::String(value) => {
            if let Some(key) = input_reference(value) {
                references.insert(key.to_string());
            }
        }
        _ => {}
    }
}

fn replace_input_references(
    value: &mut YamlValue,
    inputs: &BTreeMap<String, String>,
    location: SubstitutionLocation,
) -> Result<(), TgError> {
    match value {
        YamlValue::Sequence(values) => {
            let child_location = if location == SubstitutionLocation::Nodes {
                SubstitutionLocation::Node
            } else {
                SubstitutionLocation::Other
            };
            for value in values {
                replace_input_references(value, inputs, child_location)?;
            }
        }
        YamlValue::Mapping(values) => {
            let mut resolved = serde_yaml::Mapping::new();
            for (mut key, mut value) in std::mem::take(values) {
                replace_input_references(&mut key, inputs, SubstitutionLocation::Other)?;
                let key_name = key.as_str();
                if location != SubstitutionLocation::Node || key_name != Some("verify") {
                    let child_location =
                        if location == SubstitutionLocation::Root && key_name == Some("nodes") {
                            SubstitutionLocation::Nodes
                        } else {
                            SubstitutionLocation::Other
                        };
                    replace_input_references(&mut value, inputs, child_location)?;
                }
                if resolved.insert(key.clone(), value).is_some() {
                    let key = key
                        .as_str()
                        .map(|key| format!("'{key}'"))
                        .unwrap_or_else(|| format!("{key:?}"));
                    return Err(invalid(format!(
                        "workflow input substitution creates duplicate mapping key {key}"
                    )));
                }
            }
            *values = resolved;
        }
        YamlValue::String(value) => {
            if let Some(replacement) = input_reference(value).and_then(|key| inputs.get(key)) {
                *value = replacement.clone();
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolved_scalar<'a>(
    value: &'a YamlValue,
    inputs: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    let value = value.as_str()?;
    input_reference(value)
        .and_then(|key| inputs.get(key).map(String::as_str))
        .or(Some(value))
}

fn input_reference(value: &str) -> Option<&str> {
    let key = value.strip_prefix("${")?.strip_suffix('}')?;
    if key.is_empty() || key.contains(['{', '}']) {
        return None;
    }
    Some(key)
}

fn validate_template_header(template: &TemplateFile) -> Result<(), TgError> {
    validate_version(template.version, "workflow template")?;
    validate_nonempty(&template.name, "workflow template name")?;
    if template.plugins.is_empty() {
        return Err(invalid(
            "workflow template must declare at least one plugin",
        ));
    }
    if template.nodes.is_empty() {
        return Err(invalid("workflow template must declare nodes"));
    }
    for alias in template.plugins.keys() {
        validate_nonempty(alias, "workflow plugin alias")?;
    }
    Ok(())
}

fn load_plugins(
    workspace_root: &Path,
    plugin_paths: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, PluginDefinition>, TgError> {
    plugin_paths
        .iter()
        .map(|(alias, plugin_path)| {
            validate_nonempty(plugin_path, &format!("workflow plugin '{alias}' path"))?;
            let definition_path = canonical_definition_path(
                workspace_root,
                Path::new(plugin_path),
                &format!("plugin '{alias}'"),
            )?;
            let source = fs::read_to_string(&definition_path).map_err(TgError::IoError)?;
            let plugin: PluginFile = serde_yaml::from_str(&source)
                .map_err(|error| invalid(format!("invalid plugin '{alias}': {error}")))?;
            validate_version(plugin.version, "plugin")?;
            validate_argv(&plugin.argv, "plugin argv")?;
            Ok((
                alias.clone(),
                PluginDefinition {
                    version: plugin.version,
                    argv: plugin.argv,
                    definition_path,
                },
            ))
        })
        .collect()
}

fn validate_nodes(
    nodes: Vec<TemplateNode>,
    plugins: &BTreeMap<String, PluginDefinition>,
) -> Result<Vec<WorkflowNode>, TgError> {
    let nodes_by_id = index_nodes(&nodes)?;
    validate_parent_graph(&nodes, &nodes_by_id)?;
    validate_dependency_graph(&nodes, &nodes_by_id)?;
    let hierarchy = build_hierarchy_facts(&nodes)?;
    validate_contexts(&nodes, &nodes_by_id, plugins, &hierarchy)?;
    validate_container_descendants(&nodes, &hierarchy)?;

    nodes.into_iter().map(normalize_node).collect()
}

fn index_nodes(nodes: &[TemplateNode]) -> Result<BTreeMap<&str, &TemplateNode>, TgError> {
    let mut nodes_by_id = BTreeMap::new();
    for node in nodes {
        validate_nonempty(&node.id, "workflow node id")?;
        Item::validate_title(&node.title)?;
        if nodes_by_id.insert(node.id.as_str(), node).is_some() {
            return Err(invalid(format!("duplicate workflow node id '{}'", node.id)));
        }
    }
    Ok(nodes_by_id)
}

fn validate_parent_graph(
    nodes: &[TemplateNode],
    nodes_by_id: &BTreeMap<&str, &TemplateNode>,
) -> Result<(), TgError> {
    for node in nodes {
        if let Some(parent_id) = &node.parent {
            let parent = nodes_by_id.get(parent_id.as_str()).ok_or_else(|| {
                invalid(format!(
                    "workflow node '{}' references missing parent '{parent_id}'",
                    node.id
                ))
            })?;
            if parent.kind != NodeKind::Container {
                return Err(invalid(format!(
                    "workflow node '{}' parent '{parent_id}' is not a container",
                    node.id
                )));
            }
        }
    }
    validate_acyclic(
        nodes,
        |node| node.parent.iter().map(String::as_str),
        "parent",
    )?;

    let roots: Vec<&TemplateNode> = nodes.iter().filter(|node| node.parent.is_none()).collect();
    if roots.len() != 1 || roots[0].kind != NodeKind::Container {
        return Err(invalid(
            "workflow template must have exactly one root container",
        ));
    }
    Ok(())
}

fn validate_dependency_graph(
    nodes: &[TemplateNode],
    nodes_by_id: &BTreeMap<&str, &TemplateNode>,
) -> Result<(), TgError> {
    for node in nodes {
        if node.kind == NodeKind::Container && node.depends_on.is_some() {
            return Err(invalid(format!(
                "container '{}' cannot declare depends_on",
                node.id
            )));
        }
        let mut dependencies = BTreeSet::new();
        for dependency_id in node.depends_on.iter().flatten() {
            if !dependencies.insert(dependency_id) {
                return Err(invalid(format!(
                    "task '{}' has duplicate dependency '{dependency_id}'",
                    node.id
                )));
            }
            let dependency = nodes_by_id.get(dependency_id.as_str()).ok_or_else(|| {
                invalid(format!(
                    "task '{}' references missing dependency '{dependency_id}'",
                    node.id
                ))
            })?;
            if dependency.kind != NodeKind::Task {
                return Err(invalid(format!(
                    "task '{}' dependency '{dependency_id}' is not a task",
                    node.id
                )));
            }
        }
    }
    validate_acyclic(
        nodes,
        |node| node.depends_on.iter().flatten().map(String::as_str),
        "dependency",
    )
}

fn validate_acyclic<'a, F, I>(
    nodes: &'a [TemplateNode],
    edges: F,
    graph_name: &str,
) -> Result<(), TgError>
where
    F: Fn(&'a TemplateNode) -> I,
    I: Iterator<Item = &'a str>,
{
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisitState {
        Visiting,
        Visited,
    }

    let nodes_by_id: BTreeMap<&str, &TemplateNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut states = BTreeMap::new();
    for node in nodes {
        if states.get(node.id.as_str()) == Some(&VisitState::Visited) {
            continue;
        }

        let mut stack = vec![(node, false)];
        while let Some((current, is_exit)) = stack.pop() {
            if is_exit {
                states.insert(current.id.as_str(), VisitState::Visited);
                continue;
            }

            match states.get(current.id.as_str()) {
                Some(VisitState::Visited) => continue,
                Some(VisitState::Visiting) => {
                    return Err(invalid(format!(
                        "workflow {graph_name} cycle includes '{}'",
                        current.id
                    )));
                }
                None => {}
            }

            states.insert(current.id.as_str(), VisitState::Visiting);
            stack.push((current, true));
            let targets: Vec<&TemplateNode> = edges(current)
                .filter_map(|edge| nodes_by_id.get(edge).copied())
                .collect();
            stack.extend(targets.into_iter().rev().map(|target| (target, false)));
        }
    }
    Ok(())
}

struct HierarchyFacts<'a> {
    spans: BTreeMap<&'a str, (usize, usize)>,
    containers_with_tasks: BTreeSet<&'a str>,
}

impl HierarchyFacts<'_> {
    fn is_ancestor(&self, ancestor_id: &str, node_id: &str) -> bool {
        let Some(&(ancestor_start, ancestor_end)) = self.spans.get(ancestor_id) else {
            return false;
        };
        let Some(&(node_start, _)) = self.spans.get(node_id) else {
            return false;
        };
        ancestor_start < node_start && node_start <= ancestor_end
    }
}

fn build_hierarchy_facts(nodes: &[TemplateNode]) -> Result<HierarchyFacts<'_>, TgError> {
    let root = nodes
        .iter()
        .find(|node| node.parent.is_none())
        .ok_or_else(|| invalid("workflow template must have exactly one root container"))?;
    let mut children = BTreeMap::<&str, Vec<&TemplateNode>>::new();
    for node in nodes {
        if let Some(parent_id) = node.parent.as_deref() {
            children.entry(parent_id).or_default().push(node);
        }
    }

    let mut spans = BTreeMap::<&str, (usize, usize)>::new();
    let mut subtree_has_tasks = BTreeMap::new();
    let mut containers_with_tasks = BTreeSet::new();
    let mut preorder = 0;
    let mut stack = vec![(root, false)];
    while let Some((node, is_exit)) = stack.pop() {
        if is_exit {
            let has_task = node.kind == NodeKind::Task
                || children.get(node.id.as_str()).is_some_and(|children| {
                    children
                        .iter()
                        .any(|child| subtree_has_tasks[child.id.as_str()])
                });
            subtree_has_tasks.insert(node.id.as_str(), has_task);
            if node.kind == NodeKind::Container && has_task {
                containers_with_tasks.insert(node.id.as_str());
            }
            spans.get_mut(node.id.as_str()).unwrap().1 = preorder;
            continue;
        }

        preorder += 1;
        spans.insert(node.id.as_str(), (preorder, preorder));
        stack.push((node, true));
        if let Some(children) = children.get(node.id.as_str()) {
            stack.extend(children.iter().rev().map(|child| (*child, false)));
        }
    }

    Ok(HierarchyFacts {
        spans,
        containers_with_tasks,
    })
}

fn validate_contexts(
    nodes: &[TemplateNode],
    nodes_by_id: &BTreeMap<&str, &TemplateNode>,
    plugins: &BTreeMap<String, PluginDefinition>,
    hierarchy: &HierarchyFacts<'_>,
) -> Result<(), TgError> {
    let mut shared_plugins = BTreeMap::<String, String>::new();
    for node in nodes {
        if node.kind == NodeKind::Container {
            validate_container_fields(node)?;
            continue;
        }
        let plugin = node
            .plugin
            .as_deref()
            .ok_or_else(|| invalid(format!("task '{}' must declare plugin", node.id)))?;
        if !plugins.contains_key(plugin) {
            return Err(invalid(format!(
                "task '{}' references unknown plugin '{plugin}'",
                node.id
            )));
        }
        let context = parse_context(
            node.context
                .as_deref()
                .ok_or_else(|| invalid(format!("task '{}' must declare context", node.id)))?,
        )?;
        if let Some(key) = context.key.as_deref() {
            let is_ancestor_container = nodes_by_id
                .get(key)
                .is_some_and(|ancestor| ancestor.kind == NodeKind::Container)
                && hierarchy.is_ancestor(key, &node.id);
            if !is_ancestor_container {
                return Err(invalid(format!(
                    "task '{}' shared context '{key}' is not an ancestor container",
                    node.id
                )));
            }
            if let Some(existing_plugin) =
                shared_plugins.insert(key.to_string(), plugin.to_string())
                && existing_plugin != plugin
            {
                return Err(invalid(format!(
                    "shared context '{key}' uses multiple plugins '{existing_plugin}' and '{plugin}'"
                )));
            }
        }
        if let Some(verify) = &node.verify {
            validate_argv(verify, &format!("task '{}' verify argv", node.id))?;
        }
    }
    Ok(())
}

fn validate_container_fields(node: &TemplateNode) -> Result<(), TgError> {
    for (is_declared, field) in [
        (node.plugin.is_some(), "plugin"),
        (node.context.is_some(), "context"),
        (node.input.is_some(), "input"),
        (node.verify.is_some(), "verify"),
    ] {
        if is_declared {
            return Err(invalid(format!(
                "container '{}' cannot declare {field}",
                node.id
            )));
        }
    }
    Ok(())
}

fn validate_container_descendants(
    nodes: &[TemplateNode],
    hierarchy: &HierarchyFacts<'_>,
) -> Result<(), TgError> {
    for container in nodes.iter().filter(|node| node.kind == NodeKind::Container) {
        if !hierarchy
            .containers_with_tasks
            .contains(container.id.as_str())
        {
            return Err(invalid(format!(
                "container '{}' has no executable descendant",
                container.id
            )));
        }
    }
    Ok(())
}

fn normalize_node(node: TemplateNode) -> Result<WorkflowNode, TgError> {
    let context = node.context.as_deref().map(parse_context).transpose()?;
    Ok(WorkflowNode {
        id: node.id,
        kind: node.kind,
        parent: node.parent,
        title: node.title,
        description: node.description,
        plugin: node.plugin,
        context,
        input: node.input.unwrap_or_default(),
        verify: node.verify,
        depends_on: node.depends_on.unwrap_or_default(),
    })
}

fn parse_context(context: &str) -> Result<WorkflowContext, TgError> {
    match context {
        "fresh" => Ok(WorkflowContext {
            mode: ContextMode::Fresh,
            key: None,
        }),
        "none" => Ok(WorkflowContext {
            mode: ContextMode::None,
            key: None,
        }),
        _ => {
            let key = context.strip_prefix("shared:").ok_or_else(|| {
                invalid(format!(
                    "invalid workflow context '{context}': expected fresh, none, or shared:<container>"
                ))
            })?;
            validate_nonempty(key, "shared workflow context key")?;
            Ok(WorkflowContext {
                mode: ContextMode::Shared,
                key: Some(key.to_string()),
            })
        }
    }
}

fn validate_request_context(context: &PluginRequestContext) -> Result<(), TgError> {
    if let Some(key) = &context.key {
        validate_nonempty(key, "plugin context key")?;
    }
    if let Some(session_ref) = &context.session_ref {
        validate_nonempty(session_ref, "plugin context session_ref")?;
    }
    match context.mode {
        ContextMode::Shared => {
            if context.key.is_none() {
                return Err(invalid("shared plugin context requires a key"));
            }
            if context.resume && context.session_ref.is_none() {
                return Err(invalid(
                    "resumed shared plugin context requires a session_ref",
                ));
            }
        }
        ContextMode::Fresh => {
            if context.key.is_some() || context.session_ref.is_some() || context.resume {
                return Err(invalid(
                    "fresh plugin context cannot include key, session_ref, or resume",
                ));
            }
        }
        ContextMode::None => {
            if context.key.is_some() || context.session_ref.is_some() || context.resume {
                return Err(invalid(
                    "none plugin context cannot include key, session_ref, or resume",
                ));
            }
        }
    }
    Ok(())
}

fn validate_argv(argv: &[String], name: &str) -> Result<(), TgError> {
    let Some(program) = argv.first() else {
        return Err(invalid(format!("{name} cannot be empty")));
    };
    if program.trim().is_empty() {
        return Err(invalid(format!("{name} program cannot be empty")));
    }
    if argv.iter().any(|argument| argument.contains('\0')) {
        return Err(invalid(format!("{name} contains an argument with NUL")));
    }
    Ok(())
}

fn validate_version(version: u32, contract: &str) -> Result<(), TgError> {
    if version != WORKFLOW_VERSION {
        return Err(invalid(format!(
            "{contract} version must be 1, got {version}"
        )));
    }
    Ok(())
}

fn validate_nonempty(value: &str, name: &str) -> Result<(), TgError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{name} cannot be empty")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> TgError {
    TgError::InvalidInput(message.into())
}
