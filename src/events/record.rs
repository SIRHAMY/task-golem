//! Event record types and constructors.
//!
//! An [`Event`] is one JSON line in `events.jsonl`. The on-disk schema is
//! versioned by the `v` field; readers forward-ignore unknown versions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::status::Status;

/// Current on-disk schema version for events. Writers always emit this value;
/// readers tolerate unknown versions by skipping the record.
pub const CURRENT_EVENT_SCHEMA_VERSION: u32 = 1;

/// Closed enum of event kinds. Adding a variant is a schema-breaking change
/// and requires a bump of [`CURRENT_EVENT_SCHEMA_VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    StatusTransition,
    Note,
}

/// A single event record, one JSON line in `events.jsonl`.
///
/// The `status` field is present only when `event_type == StatusTransition`.
/// The `ts` field is serialized as RFC3339 UTC with six-digit microsecond
/// precision (e.g. `2026-04-15T14:30:22.123456Z`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Schema version. Current = [`CURRENT_EVENT_SCHEMA_VERSION`].
    pub v: u32,
    /// Task identifier (e.g. `tg-ab12c`).
    pub task_id: String,
    /// Timestamp of the event in UTC with microsecond precision on the wire.
    #[serde(
        serialize_with = "serialize_ts_microseconds",
        deserialize_with = "deserialize_ts"
    )]
    pub ts: DateTime<Utc>,
    /// Event author, as resolved by [`crate::events::author::resolve`].
    pub author: String,
    /// Event kind. Serialized as snake_case string under the JSON field
    /// `type`.
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// Free-form text payload. Empty string is permitted at the library layer
    /// (the CLI may enforce non-empty for notes).
    pub text: String,
    /// Target status. Present only for `StatusTransition` events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Event {
    /// Construct a `status_transition` event stamped with the current UTC time.
    pub fn status_transition(
        task_id: impl Into<String>,
        author: impl Into<String>,
        status: Status,
        text: impl Into<String>,
    ) -> Self {
        Self {
            v: CURRENT_EVENT_SCHEMA_VERSION,
            task_id: task_id.into(),
            ts: Utc::now(),
            author: author.into(),
            event_type: EventType::StatusTransition,
            text: text.into(),
            status: Some(status),
        }
    }

    /// Construct a `note` event stamped with the current UTC time.
    pub fn note(
        task_id: impl Into<String>,
        author: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            v: CURRENT_EVENT_SCHEMA_VERSION,
            task_id: task_id.into(),
            ts: Utc::now(),
            author: author.into(),
            event_type: EventType::Note,
            text: text.into(),
            status: None,
        }
    }
}

/// Minimal prelude used by the reader to peek at the schema version before
/// attempting a full deserialize. Unknown `v` values are skipped.
#[derive(Debug, Deserialize)]
pub(crate) struct EventPrelude {
    pub v: u32,
}

fn serialize_ts_microseconds<S>(ts: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Pin to six-digit microsecond precision. chrono's default serialization
    // uses nanoseconds when they are non-zero, which violates the on-disk
    // contract (see PRD Must-Have: "RFC3339 UTC with microsecond precision").
    let formatted = ts.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    s.serialize_str(&formatted)
}

fn deserialize_ts<'de, D>(d: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(d)?;
    // Accept any precision on read; writers emit microsecond precision but
    // third-party writers or hand-edited files may use more/less.
    DateTime::parse_from_rfc3339(&raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_transition_roundtrip() {
        let event = Event::status_transition(
            "tg-abc00",
            "alice@example.com",
            Status::Blocked,
            "needs browser verification",
        );
        let serialized = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.task_id, "tg-abc00");
        assert_eq!(parsed.author, "alice@example.com");
        assert_eq!(parsed.event_type, EventType::StatusTransition);
        assert_eq!(parsed.status, Some(Status::Blocked));
        assert_eq!(parsed.text, "needs browser verification");
        assert_eq!(parsed.v, CURRENT_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn note_roundtrip() {
        let event = Event::note("tg-abc00", "bob@example.com", "tried approach X");
        let serialized = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.event_type, EventType::Note);
        assert_eq!(parsed.status, None);
        assert_eq!(parsed.text, "tried approach X");
    }

    #[test]
    fn status_field_omitted_for_note() {
        let event = Event::note("tg-abc00", "alice", "hello");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(
            !serialized.contains("\"status\""),
            "note should omit status field: {}",
            serialized
        );
    }

    #[test]
    fn status_field_present_for_transition() {
        let event = Event::status_transition("tg-abc00", "alice", Status::Doing, "starting");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(
            serialized.contains("\"status\":\"doing\""),
            "transition must include status: {}",
            serialized
        );
    }

    #[test]
    fn type_field_renamed() {
        let event = Event::note("tg-abc00", "alice", "hi");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(
            serialized.contains("\"type\":\"note\""),
            "event type should serialize under `type`: {}",
            serialized
        );
        assert!(
            !serialized.contains("\"event_type\""),
            "internal field name should not leak: {}",
            serialized
        );
    }

    #[test]
    fn ts_serializes_with_microsecond_precision() {
        let event = Event::note("tg-abc00", "alice", "hi");
        let serialized = serde_json::to_string(&event).unwrap();

        // Extract the ts field value and verify it matches
        // `YYYY-MM-DDTHH:MM:SS.uuuuuuZ` (exactly 6 fractional digits).
        let ts_marker = "\"ts\":\"";
        let start = serialized
            .find(ts_marker)
            .expect("ts field must be present")
            + ts_marker.len();
        let end = start
            + serialized[start..]
                .find('"')
                .expect("ts field must be closed");
        let ts_value = &serialized[start..end];

        // Format: 4-2-2T2:2:2.6Z — total length 27.
        assert_eq!(
            ts_value.len(),
            27,
            "ts should be 27 chars (microsecond precision): {}",
            ts_value
        );
        let bytes = ts_value.as_bytes();
        assert_eq!(bytes[4], b'-', "year-month separator: {}", ts_value);
        assert_eq!(bytes[7], b'-', "month-day separator: {}", ts_value);
        assert_eq!(bytes[10], b'T', "date-time separator: {}", ts_value);
        assert_eq!(bytes[13], b':', "hour-minute separator: {}", ts_value);
        assert_eq!(bytes[16], b':', "minute-second separator: {}", ts_value);
        assert_eq!(bytes[19], b'.', "second-fraction separator: {}", ts_value);
        assert_eq!(bytes[26], b'Z', "UTC marker: {}", ts_value);
        // Digits 20..26 are the six fractional digits.
        for (i, b) in bytes[20..26].iter().enumerate() {
            assert!(
                b.is_ascii_digit(),
                "fractional digit {} must be a digit: {}",
                i,
                ts_value
            );
        }
    }

    #[test]
    fn ts_roundtrip_preserves_microseconds() {
        let event = Event::note("tg-abc00", "alice", "hi");
        let serialized = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&serialized).unwrap();
        // Microsecond resolution: no sub-microsecond information expected after
        // roundtrip. Allow the parsed time to match the serialized form.
        let reserialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(serialized, reserialized);
    }

    #[test]
    fn deserialize_tolerates_nanosecond_precision() {
        let line = r#"{"v":1,"task_id":"tg-abc00","ts":"2026-04-15T14:30:22.123456789Z","author":"a","type":"note","text":"x"}"#;
        let parsed: Event = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.task_id, "tg-abc00");
    }

    #[test]
    fn empty_text_permitted_at_library_layer() {
        let event = Event::note("tg-abc00", "alice", "");
        let serialized = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.text, "");
    }
}
