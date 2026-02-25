use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::TgError;
use crate::model::item::Item;

const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SchemaHeader {
    schema_version: u32,
}

/// Read items from a JSONL file (active store — fail-fast on malformed lines).
pub fn read_active(path: &Path) -> Result<Vec<Item>, TgError> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let file = fs::File::open(path).map_err(TgError::IoError)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // Parse schema header
    let header_line = match lines.next() {
        Some(Ok(line)) => line,
        Some(Err(e)) => return Err(TgError::IoError(e)),
        None => return Ok(vec![]), // Empty file
    };

    let header: SchemaHeader = serde_json::from_str(&header_line).map_err(|e| {
        TgError::StorageCorruption(format!("Invalid schema header: {}", e))
    })?;

    if header.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(TgError::SchemaVersionUnsupported {
            found: header.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let mut items = Vec::new();
    for (i, line_result) in lines.enumerate() {
        let line = line_result.map_err(TgError::IoError)?;
        if line.trim().is_empty() {
            continue;
        }
        let item: Item = serde_json::from_str(&line).map_err(|e| {
            TgError::StorageCorruption(format!("Malformed item on line {}: {}", i + 2, e))
        })?;
        items.push(item);
    }

    Ok(items)
}

/// Read items from the archive JSONL file (skip-and-warn on malformed lines).
pub fn read_archive(path: &Path) -> Result<Vec<Item>, TgError> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let file = fs::File::open(path).map_err(TgError::IoError)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // Parse schema header
    let header_line = match lines.next() {
        Some(Ok(line)) => line,
        Some(Err(e)) => return Err(TgError::IoError(e)),
        None => return Ok(vec![]),
    };

    let header: SchemaHeader = serde_json::from_str(&header_line).map_err(|e| {
        TgError::StorageCorruption(format!("Invalid archive schema header: {}", e))
    })?;

    if header.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(TgError::SchemaVersionUnsupported {
            found: header.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let mut items = Vec::new();
    for (i, line_result) in lines.enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "Warning: could not read archive line {}: {}",
                    i + 2,
                    e
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Item>(&line) {
            Ok(item) => items.push(item),
            Err(e) => {
                eprintln!(
                    "Warning: skipping malformed archive line {}: {}",
                    i + 2,
                    e
                );
            }
        }
    }

    Ok(items)
}

/// Write items to a JSONL file atomically (tempfile → fsync → rename).
/// Items are sorted by ID for deterministic output.
pub fn write_atomic(path: &Path, items: &[Item]) -> Result<(), TgError> {
    let dir = path
        .parent()
        .ok_or_else(|| TgError::IoError(std::io::Error::other(
            "Cannot determine parent directory for atomic write",
        )))?;

    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(TgError::IoError)?;

    // Write schema header
    let header = SchemaHeader {
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    writeln!(tmp, "{}", serde_json::to_string(&header).unwrap()).map_err(TgError::IoError)?;

    // Sort items by ID for deterministic output
    let mut sorted_items: Vec<&Item> = items.iter().collect();
    sorted_items.sort_by(|a, b| a.id.cmp(&b.id));

    for item in sorted_items {
        writeln!(tmp, "{}", serde_json::to_string(item).unwrap()).map_err(TgError::IoError)?;
    }

    // fsync before rename — this is the critical durability guarantee
    tmp.as_file().sync_all().map_err(TgError::IoError)?;

    // Atomic rename
    tmp.persist(path).map_err(|e| TgError::IoError(e.error))?;

    Ok(())
}

/// Append a single item to the archive file with fsync.
///
/// The archive file must already exist with a schema header (created by `tg init`).
/// If the file is missing or empty, writes the schema header first.
pub fn append_to_archive(path: &Path, item: &Item) -> Result<(), TgError> {
    // If the file doesn't exist or is empty, write schema header first
    let needs_header = !path.exists() || fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(TgError::IoError)?;

    if needs_header {
        let header = serde_json::json!({"schema_version": CURRENT_SCHEMA_VERSION});
        writeln!(file, "{}", header).map_err(TgError::IoError)?;
    }

    writeln!(file, "{}", serde_json::to_string(item).expect("Item serialization cannot fail")).map_err(TgError::IoError)?;
    file.sync_all().map_err(TgError::IoError)?;

    Ok(())
}

/// Write an empty JSONL file with just the schema header.
pub fn write_empty(path: &Path) -> Result<(), TgError> {
    write_atomic(path, &[])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;
    use crate::model::status::Status;

    fn make_item(id: &str, title: &str) -> Item {
        let now = Utc::now();
        Item {
            id: id.to_string(),
            title: title.to_string(),
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
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip_write_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let items = vec![
            make_item("tg-bbb00", "Second"),
            make_item("tg-aaa00", "First"),
        ];

        write_atomic(&path, &items).unwrap();
        let loaded = read_active(&path).unwrap();

        // Items should be sorted by ID
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "tg-aaa00");
        assert_eq!(loaded[1].id, "tg-bbb00");
    }

    #[test]
    fn schema_version_reject_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");
        fs::write(&path, "{\"schema_version\":0}\n").unwrap();

        let result = read_active(&path);
        assert!(matches!(
            result,
            Err(TgError::SchemaVersionUnsupported {
                found: 0,
                supported: 1
            })
        ));
    }

    #[test]
    fn schema_version_reject_two() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");
        fs::write(&path, "{\"schema_version\":2}\n").unwrap();

        let result = read_active(&path);
        assert!(matches!(
            result,
            Err(TgError::SchemaVersionUnsupported {
                found: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn active_malformed_line_fails_with_line_number() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let item = make_item("tg-aaa00", "Good item");
        let good_line = serde_json::to_string(&item).unwrap();
        let content = format!("{{\"schema_version\":1}}\n{}\n{{bad json\n", good_line);
        fs::write(&path, content).unwrap();

        let result = read_active(&path);
        match result {
            Err(TgError::StorageCorruption(msg)) => {
                assert!(msg.contains("line 3"), "Should mention line 3: {}", msg);
            }
            other => panic!("Expected StorageCorruption, got: {:?}", other),
        }
    }

    #[test]
    fn archive_malformed_line_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");

        let item = make_item("tg-aaa00", "Good item");
        let good_line = serde_json::to_string(&item).unwrap();
        let content = format!("{{\"schema_version\":1}}\n{}\n{{bad json\n", good_line);
        fs::write(&path, content).unwrap();

        let items = read_archive(&path).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "tg-aaa00");
    }

    #[test]
    fn items_sorted_by_id_in_output() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let items = vec![
            make_item("tg-zzz00", "Z"),
            make_item("tg-aaa00", "A"),
            make_item("tg-mmm00", "M"),
        ];

        write_atomic(&path, &items).unwrap();
        let loaded = read_active(&path).unwrap();
        assert_eq!(loaded[0].id, "tg-aaa00");
        assert_eq!(loaded[1].id, "tg-mmm00");
        assert_eq!(loaded[2].id, "tg-zzz00");
    }

    #[test]
    fn archive_truncated_last_line_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");

        let item = make_item("tg-aaa00", "Good item");
        let good_line = serde_json::to_string(&item).unwrap();
        // Truncated JSON on last line (simulate crash mid-append)
        let content = format!(
            "{{\"schema_version\":1}}\n{}\n{{\"id\":\"tg-bbb00\",\"tit",
            good_line
        );
        fs::write(&path, content).unwrap();

        let items = read_archive(&path).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "tg-aaa00");
    }

    #[test]
    fn empty_file_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");
        write_empty(&path).unwrap();

        let items = read_active(&path).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn nonexistent_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent.jsonl");
        let items = read_active(&path).unwrap();
        assert!(items.is_empty());
    }
}
