//! Lenient JSONL reader for event records.
//!
//! Mirrors the forgiving behavior of [`crate::store::jsonl::read_archive`]:
//! malformed lines (truncated tails, partial writes) warn to stderr and are
//! skipped. Unknown schema versions (`v`) are skipped silently for
//! forward-compatibility. Results are sorted by timestamp ascending.
//!
//! Each line is parsed in two steps:
//!
//! 1. Deserialize a minimal prelude (`{ "v": u32 }`) and check the schema
//!    version. Unknown versions are skipped without attempting to decode the
//!    full payload (which may use a `type` variant we don't know about).
//! 2. Deserialize the full [`Event`]. Failures here warn and skip.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::errors::TgError;
use crate::events::record::{CURRENT_EVENT_SCHEMA_VERSION, Event, EventPrelude};

/// Read all events from `path`, sorted by timestamp ascending. Missing file
/// returns an empty vec; malformed lines warn-and-skip; unknown schema
/// versions are silently skipped.
pub fn all(path: &Path) -> Result<Vec<Event>, TgError> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let file = fs::File::open(path).map_err(TgError::IoError)?;
    let reader = BufReader::new(file);
    let mut events = read_from_reader(reader, path);
    events.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(events)
}

/// Read events belonging to `task_id` from `path`, sorted by timestamp
/// ascending.
pub fn for_task(path: &Path, task_id: &str) -> Result<Vec<Event>, TgError> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let file = fs::File::open(path).map_err(TgError::IoError)?;
    let reader = BufReader::new(file);
    let mut events: Vec<Event> = read_from_reader(reader, path)
        .into_iter()
        .filter(|e| e.task_id == task_id)
        .collect();
    events.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(events)
}

/// Parse each line, skipping malformed lines with a single warning emission.
/// Unknown `v` values are skipped silently (no warning).
fn read_from_reader<R: BufRead>(reader: R, path: &Path) -> Vec<Event> {
    let mut events = Vec::new();
    let mut warned = false;
    for (i, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                warn_once(
                    &mut warned,
                    path,
                    &format!("could not read line {}: {}", i + 1, e),
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        // Prelude-first: check schema version before attempting full parse.
        let prelude: EventPrelude = match serde_json::from_str(&line) {
            Ok(p) => p,
            Err(e) => {
                warn_once(
                    &mut warned,
                    path,
                    &format!("skipping malformed line {}: {}", i + 1, e),
                );
                continue;
            }
        };
        if prelude.v != CURRENT_EVENT_SCHEMA_VERSION {
            // Forward-compat: silently skip records from newer (or older,
            // unsupported) schema versions. No warning.
            continue;
        }

        match serde_json::from_str::<Event>(&line) {
            Ok(event) => events.push(event),
            Err(e) => {
                warn_once(
                    &mut warned,
                    path,
                    &format!("skipping malformed line {}: {}", i + 1, e),
                );
            }
        }
    }
    events
}

fn warn_once(warned: &mut bool, path: &Path, msg: &str) {
    if !*warned {
        eprintln!("Warning: {}: {}", path.display(), msg);
        *warned = true;
    } else {
        eprintln!("  (also) {}", msg);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::events::append;
    use crate::events::record::Event;
    use crate::model::status::Status;

    #[test]
    fn missing_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        assert!(all(&path).unwrap().is_empty());
        assert!(for_task(&path, "tg-abc00").unwrap().is_empty());
    }

    #[test]
    fn empty_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        fs::write(&path, "").unwrap();
        assert!(all(&path).unwrap().is_empty());
    }

    #[test]
    fn single_event_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let event = Event::note("tg-abc00", "alice", "hello");
        append::write(&path, &event).unwrap();

        let read_back = all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].task_id, "tg-abc00");
        assert_eq!(read_back[0].text, "hello");
    }

    #[test]
    fn for_task_filters_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        append::write(&path, &Event::note("tg-aaa00", "alice", "a")).unwrap();
        append::write(&path, &Event::note("tg-bbb00", "alice", "b")).unwrap();
        append::write(&path, &Event::note("tg-aaa00", "alice", "a2")).unwrap();

        let a_events = for_task(&path, "tg-aaa00").unwrap();
        assert_eq!(a_events.len(), 2);
        assert!(a_events.iter().all(|e| e.task_id == "tg-aaa00"));

        let b_events = for_task(&path, "tg-bbb00").unwrap();
        assert_eq!(b_events.len(), 1);
    }

    #[test]
    fn results_sorted_by_ts_ascending() {
        // Write events with manually crafted ts out of order.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");

        let later = r#"{"v":1,"task_id":"tg-abc00","ts":"2026-04-15T14:30:22.000000Z","author":"a","type":"note","text":"later"}"#;
        let earlier = r#"{"v":1,"task_id":"tg-abc00","ts":"2026-04-15T14:29:22.000000Z","author":"a","type":"note","text":"earlier"}"#;
        fs::write(&path, format!("{}\n{}\n", later, earlier)).unwrap();

        let events = all(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "earlier");
        assert_eq!(events[1].text, "later");
    }

    #[test]
    fn malformed_trailing_line_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let good = Event::note("tg-abc00", "alice", "good");
        let good_line = serde_json::to_string(&good).unwrap();
        fs::write(&path, format!("{}\n{{truncated", good_line)).unwrap();

        let events = all(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "good");
    }

    #[test]
    fn malformed_middle_line_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let a = serde_json::to_string(&Event::note("tg-abc00", "alice", "a")).unwrap();
        let b = serde_json::to_string(&Event::note("tg-abc00", "alice", "b")).unwrap();
        fs::write(&path, format!("{}\n{{not json\n{}\n", a, b)).unwrap();

        let events = all(&path).unwrap();
        assert_eq!(events.len(), 2);
        // Both good lines present; order is by ts (both ~Utc::now() but in
        // serialization order).
        let texts: Vec<_> = events.iter().map(|e| e.text.as_str()).collect();
        assert!(texts.contains(&"a"));
        assert!(texts.contains(&"b"));
    }

    #[test]
    fn unknown_schema_version_skipped_silently() {
        // A v:99 record with a `type` we don't recognize must not cause the
        // full deserialize to error — the prelude check runs first.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let future = r#"{"v":99,"task_id":"tg-abc00","ts":"2030-01-01T00:00:00.000000Z","author":"a","type":"some_future_variant","text":"x","new_field":42}"#;
        let good = serde_json::to_string(&Event::note("tg-abc00", "alice", "good")).unwrap();
        fs::write(&path, format!("{}\n{}\n", future, good)).unwrap();

        let events = all(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "good");
    }

    #[test]
    fn status_transition_roundtrip_via_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        let event = Event::status_transition("tg-abc00", "alice", Status::Blocked, "needs review");
        append::write(&path, &event).unwrap();
        let read_back = all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].status, Some(Status::Blocked));
    }
}
