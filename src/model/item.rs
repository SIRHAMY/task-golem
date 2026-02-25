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

#[allow(dead_code)] // Used in later phases
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
}
