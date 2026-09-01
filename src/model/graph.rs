use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphApplyRequest {
    pub items: Vec<GraphApplyItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphApplyItem {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: i64,
    pub tags: Vec<String>,
    pub parent: Option<GraphRef>,
    pub dependencies: Vec<GraphRef>,
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRef {
    Local(String),
    Existing(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphApplyCategory {
    InvalidRequest,
    InvalidGraph,
    StorageCorruption,
    PersistenceFailure,
}

impl GraphApplyCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidGraph => "invalid_graph",
            Self::StorageCorruption => "storage_corruption",
            Self::PersistenceFailure => "persistence_failure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphApplyDiagnosticCode {
    EmptyGraph,
    DuplicateReference,
    MissingReference,
    InvalidId,
    SelfReference,
    DuplicateDependency,
    InvalidItem,
    ParentCycle,
    DependencyCycle,
    DuplicateDurableId,
    PersistenceFailure,
}

impl GraphApplyDiagnosticCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmptyGraph => "empty_graph",
            Self::DuplicateReference => "duplicate_reference",
            Self::MissingReference => "missing_reference",
            Self::InvalidId => "invalid_id",
            Self::SelfReference => "self_reference",
            Self::DuplicateDependency => "duplicate_dependency",
            Self::InvalidItem => "invalid_item",
            Self::ParentCycle => "parent_cycle",
            Self::DependencyCycle => "dependency_cycle",
            Self::DuplicateDurableId => "duplicate_durable_id",
            Self::PersistenceFailure => "persistence_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphApplyDiagnostic {
    pub code: GraphApplyDiagnosticCode,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<GraphRef>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, serde_json::Value>,
}

impl GraphApplyDiagnostic {
    pub fn new(code: GraphApplyDiagnosticCode, path: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            reference: None,
            details: BTreeMap::new(),
        }
    }

    pub fn with_reference(mut self, reference: GraphRef) -> Self {
        self.reference = Some(reference);
        self
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        let value = serde_json::to_value(value).expect("graph diagnostic details must serialize");
        self.details.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphApplyError {
    pub operation: GraphApplyOperation,
    pub outcome: GraphApplyErrorOutcome,
    pub category: GraphApplyCategory,
    pub diagnostics: Vec<GraphApplyDiagnostic>,
}

impl GraphApplyError {
    pub fn new(category: GraphApplyCategory, mut diagnostics: Vec<GraphApplyDiagnostic>) -> Self {
        diagnostics.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.reference.cmp(&right.reference))
                .then_with(|| diagnostic_details_key(left).cmp(&diagnostic_details_key(right)))
        });

        Self {
            operation: GraphApplyOperation::GraphApply,
            outcome: GraphApplyErrorOutcome::Error,
            category,
            diagnostics,
        }
    }

    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self::new(
            GraphApplyCategory::InvalidRequest,
            vec![
                GraphApplyDiagnostic::new(GraphApplyDiagnosticCode::InvalidItem, "$")
                    .with_detail("message", message.into()),
            ],
        )
    }

    pub fn to_json(&self, exit_code: i32) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("graph apply errors must serialize");
        value
            .as_object_mut()
            .expect("graph apply error must serialize as an object")
            .insert("exit_code".to_string(), serde_json::json!(exit_code));
        value
    }
}

impl std::fmt::Display for GraphApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "graph_apply failed with {}",
            self.category.as_str()
        )
    }
}

impl std::error::Error for GraphApplyError {}

fn diagnostic_details_key(diagnostic: &GraphApplyDiagnostic) -> String {
    serde_json::to_string(&diagnostic.details).expect("graph diagnostic details must serialize")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphApplyOperation {
    GraphApply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphApplyOutcome {
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphApplyErrorOutcome {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphApplyResult {
    pub operation: GraphApplyOperation,
    pub outcome: GraphApplyOutcome,
    pub count: usize,
    pub mapping: BTreeMap<String, String>,
}

impl GraphApplyResult {
    pub fn created(mapping: BTreeMap<String, String>) -> Self {
        Self {
            operation: GraphApplyOperation::GraphApply,
            outcome: GraphApplyOutcome::Created,
            count: mapping.len(),
            mapping,
        }
    }
}
