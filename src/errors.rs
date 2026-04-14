use std::fmt;

use serde::Serialize;

use crate::model::status::Status;

#[derive(Debug, thiserror::Error)]
pub enum TgError {
    // User errors (exit code 1)
    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Invalid transition: {from} cannot transition to {to}")]
    InvalidTransition { from: Status, to: Status },

    #[error("Ambiguous ID prefix '{prefix}': matches {matches:?}")]
    AmbiguousId {
        prefix: String,
        matches: Vec<String>,
    },

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
}

impl TgError {
    pub fn exit_code(&self) -> i32 {
        match self {
            TgError::ItemNotFound(_)
            | TgError::InvalidTransition { .. }
            | TgError::AmbiguousId { .. }
            | TgError::CycleDetected(_)
            | TgError::AlreadyClaimed(_)
            | TgError::InvalidInput(_)
            | TgError::NotInitialized(_)
            | TgError::DependentExists(_, _)
            | TgError::ParentSelfReference { .. }
            | TgError::ParentCycle { .. }
            | TgError::ParentDangling { .. }
            | TgError::ParentHasChildren { .. } => 1,

            TgError::StorageCorruption(_)
            | TgError::LockTimeout(_)
            | TgError::IoError(_)
            | TgError::IdCollisionExhausted(_)
            | TgError::SchemaVersionUnsupported { .. } => 2,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": self.to_string(),
            "exit_code": self.exit_code(),
        })
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
