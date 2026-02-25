use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};

use super::status::Status;
use crate::errors::TgError;

/// Serialize Option<T> so that None becomes JSON null (not omitted).
fn serialize_option_nullable<S, T>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    match value {
        Some(v) => v.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub priority: i64,

    #[serde(serialize_with = "serialize_option_nullable")]
    pub description: Option<String>,

    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[serde(serialize_with = "serialize_option_nullable")]
    pub blocked_reason: Option<String>,

    #[serde(serialize_with = "serialize_option_nullable")]
    pub blocked_from_status: Option<Status>,

    #[serde(serialize_with = "serialize_option_nullable")]
    pub claimed_by: Option<String>,

    #[serde(serialize_with = "serialize_option_nullable")]
    pub claimed_at: Option<DateTime<Utc>>,

    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl Item {
    pub fn validate_title(title: &str) -> Result<(), TgError> {
        if title.contains('\n') || title.contains('\r') {
            return Err(TgError::InvalidInput(
                "Title must be a single line (no newlines)".to_string(),
            ));
        }
        if title.trim().is_empty() {
            return Err(TgError::InvalidInput("Title cannot be empty".to_string()));
        }
        Ok(())
    }

    /// Transition to Doing, optionally setting claim fields.
    pub fn apply_do(&mut self, claim: Option<String>) {
        let now = Utc::now();
        self.status = Status::Doing;
        if let Some(agent) = claim {
            self.claimed_by = Some(agent);
            self.claimed_at = Some(now);
        }
        self.updated_at = now;
    }

    /// Transition to Done, clearing claim fields.
    pub fn apply_done(&mut self) {
        self.status = Status::Done;
        self.claimed_by = None;
        self.claimed_at = None;
        self.updated_at = Utc::now();
    }

    /// Transition to Blocked, storing current status for later restoration.
    pub fn apply_block(&mut self, reason: Option<String>) {
        let from_status = self.status;
        self.blocked_from_status = Some(from_status);
        self.status = Status::Blocked;
        self.blocked_reason = reason;
        // Clear claims if transitioning from Doing
        if from_status == Status::Doing {
            self.claimed_by = None;
            self.claimed_at = None;
        }
        self.updated_at = Utc::now();
    }

    /// Restore status from blocked_from_status (default to Todo if missing).
    pub fn apply_unblock(&mut self) {
        self.status = self.blocked_from_status.unwrap_or(Status::Todo);
        self.blocked_reason = None;
        self.blocked_from_status = None;
        self.updated_at = Utc::now();
    }

    /// Transition back to Todo, clearing claim fields.
    pub fn apply_todo(&mut self) {
        self.status = Status::Todo;
        self.claimed_by = None;
        self.claimed_at = None;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_item() -> Item {
        let now = DateTime::parse_from_rfc3339("2026-02-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut extensions = BTreeMap::new();
        extensions.insert(
            "x-agent".to_string(),
            serde_json::json!({"name": "test-agent", "version": "1.0"}),
        );
        extensions.insert("x-priority-label".to_string(), serde_json::json!("high"));

        Item {
            id: "tg-a1b2c".to_string(),
            title: "Test item".to_string(),
            status: Status::Todo,
            priority: 0,
            description: None,
            tags: vec!["backend".to_string()],
            dependencies: vec![],
            created_at: now,
            updated_at: now,
            blocked_reason: None,
            blocked_from_status: None,
            claimed_by: None,
            claimed_at: None,
            extensions,
        }
    }

    /// Serde PoC: verify flatten + BTreeMap + null Options produce deterministic round-trip
    #[test]
    fn serde_poc_round_trip_byte_identical() {
        let item = make_test_item();

        let json1 = serde_json::to_string(&item).unwrap();
        let deserialized: Item = serde_json::from_str(&json1).unwrap();
        let json2 = serde_json::to_string(&deserialized).unwrap();

        assert_eq!(json1, json2, "Round-trip must produce byte-identical JSON");
    }

    /// Verify None fields serialize as null, not omitted
    #[test]
    fn serde_poc_null_fields_present() {
        let item = make_test_item();
        let json = serde_json::to_string(&item).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["description"], serde_json::Value::Null);
        assert_eq!(parsed["blocked_reason"], serde_json::Value::Null);
        assert_eq!(parsed["blocked_from_status"], serde_json::Value::Null);
        assert_eq!(parsed["claimed_by"], serde_json::Value::Null);
        assert_eq!(parsed["claimed_at"], serde_json::Value::Null);
    }

    /// Verify extension fields appear after known fields with alphabetical ordering
    #[test]
    fn serde_poc_extension_ordering() {
        let item = make_test_item();
        let json = serde_json::to_string(&item).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Extension fields should be present
        assert!(parsed["x-agent"].is_object());
        assert_eq!(parsed["x-priority-label"], "high");

        // Nested extension object keys should be alphabetically ordered
        let agent = &parsed["x-agent"];
        let keys: Vec<&str> = agent.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["name", "version"]); // alphabetical (BTreeMap)
    }

    /// Verify nested Value::Object keys are alphabetically ordered
    #[test]
    fn serde_poc_nested_value_object_ordering() {
        let mut extensions = BTreeMap::new();
        extensions.insert(
            "x-meta".to_string(),
            serde_json::json!({"zebra": 1, "alpha": 2, "middle": 3}),
        );

        let now = Utc::now();
        let item = Item {
            id: "tg-test1".to_string(),
            title: "Test".to_string(),
            status: Status::Todo,
            priority: 0,
            description: None,
            tags: vec![],
            dependencies: vec![],
            created_at: now,
            updated_at: now,
            blocked_reason: None,
            blocked_from_status: None,
            claimed_by: None,
            claimed_at: None,
            extensions,
        };

        let json = serde_json::to_string(&item).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let meta_keys: Vec<&str> = parsed["x-meta"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(meta_keys, vec!["alpha", "middle", "zebra"]);
    }

    /// Verify chrono DateTime serializes as ISO 8601 UTC
    #[test]
    fn serde_chrono_iso8601() {
        let item = make_test_item();
        let json = serde_json::to_string(&item).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let created = parsed["created_at"].as_str().unwrap();
        assert_eq!(created, "2026-02-24T12:00:00Z");
    }

    #[test]
    fn title_validation_rejects_newlines() {
        assert!(Item::validate_title("Valid title").is_ok());
        assert!(Item::validate_title("Has\nnewline").is_err());
        assert!(Item::validate_title("Has\r\nnewline").is_err());
        assert!(Item::validate_title("Has\rnewline").is_err());
        assert!(Item::validate_title("  ").is_err());
        assert!(Item::validate_title("").is_err());
    }

    /// Full round-trip with Some values to verify non-null optional fields
    #[test]
    fn serde_round_trip_with_some_values() {
        let now = DateTime::parse_from_rfc3339("2026-02-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let item = Item {
            id: "tg-abc12".to_string(),
            title: "Blocked item".to_string(),
            status: Status::Blocked,
            priority: 5,
            description: Some("A description".to_string()),
            tags: vec!["urgent".to_string()],
            dependencies: vec!["tg-dep01".to_string()],
            created_at: now,
            updated_at: now,
            blocked_reason: Some("Waiting on API".to_string()),
            blocked_from_status: Some(Status::Doing),
            claimed_by: Some("agent-1".to_string()),
            claimed_at: Some(now),
            extensions: BTreeMap::new(),
        };

        let json1 = serde_json::to_string(&item).unwrap();
        let deserialized: Item = serde_json::from_str(&json1).unwrap();
        let json2 = serde_json::to_string(&deserialized).unwrap();
        assert_eq!(json1, json2);
    }

    // === apply_* method tests ===

    #[test]
    fn apply_do_without_claim() {
        let mut item = make_test_item();
        let before = item.updated_at;
        item.apply_do(None);
        assert_eq!(item.status, Status::Doing);
        assert!(item.claimed_by.is_none());
        assert!(item.claimed_at.is_none());
        assert!(item.updated_at >= before);
    }

    #[test]
    fn apply_do_with_claim() {
        let mut item = make_test_item();
        item.apply_do(Some("agent-1".to_string()));
        assert_eq!(item.status, Status::Doing);
        assert_eq!(item.claimed_by.as_deref(), Some("agent-1"));
        assert!(item.claimed_at.is_some());
        // claimed_at and updated_at should be the same instant
        assert_eq!(item.claimed_at.unwrap(), item.updated_at);
    }

    #[test]
    fn apply_done_clears_claims() {
        let mut item = make_test_item();
        item.apply_do(Some("agent-1".to_string()));
        item.apply_done();
        assert_eq!(item.status, Status::Done);
        assert!(item.claimed_by.is_none());
        assert!(item.claimed_at.is_none());
    }

    #[test]
    fn apply_block_from_todo() {
        let mut item = make_test_item();
        item.apply_block(Some("waiting".to_string()));
        assert_eq!(item.status, Status::Blocked);
        assert_eq!(item.blocked_from_status, Some(Status::Todo));
        assert_eq!(item.blocked_reason.as_deref(), Some("waiting"));
        // No claims to clear from todo
        assert!(item.claimed_by.is_none());
    }

    #[test]
    fn apply_block_from_doing_clears_claims() {
        let mut item = make_test_item();
        item.apply_do(Some("agent-1".to_string()));
        assert!(item.claimed_by.is_some());

        item.apply_block(Some("blocker".to_string()));
        assert_eq!(item.status, Status::Blocked);
        assert_eq!(item.blocked_from_status, Some(Status::Doing));
        assert!(item.claimed_by.is_none());
        assert!(item.claimed_at.is_none());
    }

    #[test]
    fn apply_unblock_restores_status() {
        let mut item = make_test_item();
        item.apply_do(None);
        item.apply_block(None);
        assert_eq!(item.blocked_from_status, Some(Status::Doing));

        item.apply_unblock();
        assert_eq!(item.status, Status::Doing);
        assert!(item.blocked_reason.is_none());
        assert!(item.blocked_from_status.is_none());
    }

    #[test]
    fn apply_unblock_defaults_to_todo() {
        let mut item = make_test_item();
        item.status = Status::Blocked;
        item.blocked_from_status = None; // Simulate corrupted data

        item.apply_unblock();
        assert_eq!(item.status, Status::Todo);
    }

    #[test]
    fn apply_todo_clears_claims() {
        let mut item = make_test_item();
        item.apply_do(Some("agent-1".to_string()));
        assert!(item.claimed_by.is_some());

        item.apply_todo();
        assert_eq!(item.status, Status::Todo);
        assert!(item.claimed_by.is_none());
        assert!(item.claimed_at.is_none());
    }
}
