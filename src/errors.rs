use std::fmt;

use serde::Serialize;

use crate::model::graph::{GraphApplyCategory, GraphApplyError};
use crate::model::status::Status;

#[derive(Debug, thiserror::Error)]
pub enum TgError {
    // User errors (exit code 1)
    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Invalid item ID: {0}")]
    InvalidId(String),

    #[error("Invalid transition: {from} cannot transition to {to}")]
    InvalidTransition { from: Status, to: Status },

    #[error("Dependency cycle detected: {0}")]
    CycleDetected(String),

    #[error("Already claimed by {0}")]
    AlreadyClaimed(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("No task-golem project found (searched from {0}). Run `tg init` to create one.")]
    NotInitialized(String),

    #[error("Item {0} is depended on by: {1}")]
    DependentExists(String, String),

    #[error("Item {item_id} depends on missing target {dependency_id}")]
    DependencyMissing {
        item_id: String,
        dependency_id: String,
    },

    #[error("Item {id} cannot be its own parent")]
    ParentSelfReference { id: String },

    #[error("Parent cycle detected among: {}", ids.join(", "))]
    ParentCycle { ids: Vec<String> },

    #[error("Parent '{parent}' for item {id} not found in active items")]
    ParentDangling { id: String, parent: String },

    #[error("Item {id} has children and cannot be removed: {}", children.join(", "))]
    ParentHasChildren { id: String, children: Vec<String> },

    // System errors (exit code 2)
    #[error("Storage corruption: {0}")]
    StorageCorruption(String),

    #[error("Lock timeout after {0:?}")]
    LockTimeout(std::time::Duration),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error("ID collision exhausted after {0} attempts")]
    IdCollisionExhausted(u32),

    #[error("Unsupported schema version {found} (max supported: {supported})")]
    SchemaVersionUnsupported { found: u32, supported: u32 },

    #[error("Cache corrupt: {detail}")]
    CacheCorrupt { detail: String },

    #[error("Cache rebuild failed: {detail}")]
    CacheRebuildFailed { detail: String },

    #[error(
        "Cache schema version mismatch (stored: {stored}, expected: {expected}); rebuild required"
    )]
    CacheSchemaVersionMismatch { stored: u32, expected: u32 },

    #[error("Query exceeded timeout of {limit_secs}s. Use --timeout N to extend.")]
    QueryTimeout { limit_secs: u64 },

    #[error("Query denied by sandbox: {action}. {hint}")]
    QueryDenied { action: String, hint: String },

    #[error("Query syntax error: {message}")]
    QuerySyntax { message: String },

    #[error(transparent)]
    GraphApply(#[from] GraphApplyError),
}

impl TgError {
    pub fn exit_code(&self) -> i32 {
        match self {
            TgError::ItemNotFound(_)
            | TgError::InvalidId(_)
            | TgError::InvalidTransition { .. }
            | TgError::CycleDetected(_)
            | TgError::AlreadyClaimed(_)
            | TgError::InvalidInput(_)
            | TgError::NotInitialized(_)
            | TgError::DependentExists(_, _)
            | TgError::DependencyMissing { .. }
            | TgError::ParentSelfReference { .. }
            | TgError::ParentCycle { .. }
            | TgError::ParentDangling { .. }
            | TgError::ParentHasChildren { .. }
            | TgError::QueryTimeout { .. }
            | TgError::QueryDenied { .. }
            | TgError::QuerySyntax { .. } => 1,

            TgError::GraphApply(error) => match error.category {
                GraphApplyCategory::InvalidRequest | GraphApplyCategory::InvalidGraph => 1,
                GraphApplyCategory::StorageCorruption | GraphApplyCategory::PersistenceFailure => 2,
            },

            TgError::StorageCorruption(_)
            | TgError::LockTimeout(_)
            | TgError::IoError(_)
            | TgError::IdCollisionExhausted(_)
            | TgError::SchemaVersionUnsupported { .. }
            | TgError::CacheCorrupt { .. }
            | TgError::CacheRebuildFailed { .. }
            | TgError::CacheSchemaVersionMismatch { .. } => 2,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        if let TgError::GraphApply(error) = self {
            return error.to_json(self.exit_code());
        }

        serde_json::json!({
            "code": self.code(),
            "error": self.to_string(),
            "exit_code": self.exit_code(),
        })
    }

    pub fn code(&self) -> &'static str {
        match self {
            TgError::ItemNotFound(_) => "item_not_found",
            TgError::InvalidId(_) => "invalid_id",
            TgError::InvalidTransition { .. } => "invalid_transition",
            TgError::CycleDetected(_) => "cycle_detected",
            TgError::AlreadyClaimed(_) => "already_claimed",
            TgError::InvalidInput(_) => "invalid_input",
            TgError::NotInitialized(_) => "not_initialized",
            TgError::DependentExists(_, _) => "dependent_exists",
            TgError::DependencyMissing { .. } => "dependency_missing",
            TgError::ParentSelfReference { .. } => "parent_self_reference",
            TgError::ParentCycle { .. } => "parent_cycle",
            TgError::ParentDangling { .. } => "parent_dangling",
            TgError::ParentHasChildren { .. } => "parent_has_children",
            TgError::StorageCorruption(_) => "storage_corruption",
            TgError::LockTimeout(_) => "lock_timeout",
            TgError::IoError(_) => "io_error",
            TgError::IdCollisionExhausted(_) => "id_collision_exhausted",
            TgError::SchemaVersionUnsupported { .. } => "schema_version_unsupported",
            TgError::CacheCorrupt { .. } => "cache_corrupt",
            TgError::CacheRebuildFailed { .. } => "cache_rebuild_failed",
            TgError::CacheSchemaVersionMismatch { .. } => "cache_schema_version_mismatch",
            TgError::QueryTimeout { .. } => "query_timeout",
            TgError::QueryDenied { .. } => "query_denied",
            TgError::QuerySyntax { .. } => "query_syntax",
            TgError::GraphApply(_) => "graph_apply",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonError {
    pub error: String,
    pub exit_code: i32,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}
