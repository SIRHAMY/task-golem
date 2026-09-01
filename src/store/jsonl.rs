use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::TgError;
use crate::model::id;
use crate::model::item::Item;

const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SchemaHeader {
    schema_version: u32,
}

#[derive(Debug, Deserialize)]
struct ItemIdentity {
    id: String,
}

pub(crate) fn validate_item_ids(items: &[Item]) -> Result<HashSet<String>, TgError> {
    let mut ids = HashSet::with_capacity(items.len());
    for item in items {
        id::validate_id(&item.id)?;
        if !ids.insert(item.id.clone()) {
            return Err(duplicate_id_error(&item.id));
        }
    }
    Ok(ids)
}

pub(crate) fn duplicate_id_error(id: &str) -> TgError {
    TgError::StorageCorruption(format!("Duplicate item ID: {id}"))
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

    let header: SchemaHeader = serde_json::from_str(&header_line)
        .map_err(|e| TgError::StorageCorruption(format!("Invalid schema header: {}", e)))?;

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
        item.validate_extensions().map_err(|e| match e {
            TgError::StorageCorruption(msg) => {
                TgError::StorageCorruption(format!("Invalid extensions on line {}: {}", i + 2, msg))
            }
            other => other,
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

    let header: SchemaHeader = serde_json::from_str(&header_line)
        .map_err(|e| TgError::StorageCorruption(format!("Invalid archive schema header: {}", e)))?;

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
                eprintln!("Warning: could not read archive line {}: {}", i + 2, e);
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Item>(&line) {
            Ok(item) => match item.validate_extensions() {
                Ok(()) => items.push(item),
                Err(TgError::StorageCorruption(msg)) => {
                    eprintln!(
                        "Warning: skipping archive item with invalid extensions on line {}: {}",
                        i + 2,
                        msg
                    );
                }
                Err(e) => {
                    eprintln!("Warning: skipping archive item on line {}: {}", i + 2, e);
                }
            },
            Err(e) => {
                eprintln!("Warning: skipping malformed archive line {}: {}", i + 2, e);
            }
        }
    }

    Ok(items)
}

/// Read every durable archive identity without requiring the rest of an item to be valid.
pub fn read_archive_ids(path: &Path) -> Result<HashSet<String>, TgError> {
    if !path.exists() {
        return Ok(HashSet::new());
    }

    let content = fs::read_to_string(path).map_err(TgError::IoError)?;
    if content.is_empty() {
        return Ok(HashSet::new());
    }

    let lines: Vec<&str> = content.lines().collect();
    let header_line = lines
        .first()
        .ok_or_else(|| TgError::StorageCorruption("Missing archive schema header".to_string()))?;
    let header: SchemaHeader = serde_json::from_str(header_line)
        .map_err(|e| TgError::StorageCorruption(format!("Invalid archive schema header: {e}")))?;
    if header.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(TgError::SchemaVersionUnsupported {
            found: header.schema_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let mut ids = HashSet::new();
    for (index, line) in lines.iter().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }

        let identity = match serde_json::from_str::<ItemIdentity>(line) {
            Ok(identity) => identity,
            Err(error) => {
                eprintln!(
                    "Warning: skipping malformed archive line {} while scanning IDs: {}",
                    index + 1,
                    error
                );
                continue;
            }
        };

        id::validate_id(&identity.id).map_err(|_| {
            TgError::StorageCorruption(format!(
                "Invalid archive item ID on line {}: {}",
                index + 1,
                identity.id
            ))
        })?;
        if !ids.insert(identity.id.clone()) {
            return Err(duplicate_id_error(&identity.id));
        }
    }

    Ok(ids)
}

/// Write items to a JSONL file atomically (tempfile → fsync → rename).
/// Items are sorted by ID for deterministic output.
pub fn write_atomic(path: &Path, items: &[Item]) -> Result<(), TgError> {
    validate_item_ids(items)?;

    let dir = path.parent().ok_or_else(|| {
        TgError::IoError(std::io::Error::other(
            "Cannot determine parent directory for atomic write",
        ))
    })?;

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
/// Handles crash recovery: if the file doesn't end with a newline (truncated write),
/// prepends a newline so the new item starts on its own line.
pub fn append_to_archive(path: &Path, item: &Item) -> Result<(), TgError> {
    id::validate_id(&item.id)?;
    let archive_ids = read_archive_ids(path)?;
    if archive_ids.contains(&item.id) {
        return Err(duplicate_id_error(&item.id));
    }

    // If the file doesn't exist or is empty, write schema header first
    let needs_header = !path.exists() || fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);

    let needs_leading_newline = if !needs_header && path.exists() {
        let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if len > 0 {
            use std::io::{Read as _, Seek, SeekFrom};
            let mut file = fs::File::open(path).map_err(TgError::IoError)?;
            file.seek(SeekFrom::End(-1)).map_err(TgError::IoError)?;
            let mut buffer = [0u8; 1];
            file.read_exact(&mut buffer).map_err(TgError::IoError)?;
            buffer[0] != b'\n'
        } else {
            false
        }
    } else {
        false
    };

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(TgError::IoError)?;

    if needs_header {
        let header = serde_json::json!({"schema_version": CURRENT_SCHEMA_VERSION});
        writeln!(file, "{}", header).map_err(TgError::IoError)?;
    }

    if needs_leading_newline {
        writeln!(file).map_err(TgError::IoError)?;
    }

    writeln!(
        file,
        "{}",
        serde_json::to_string(item).expect("Item serialization cannot fail")
    )
    .map_err(TgError::IoError)?;
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

    const ID_A: &str = "018f2b1c-4d5e-7abc-8123-000000000001";
    const ID_B: &str = "018f2b1c-4d5e-7abc-8123-000000000002";
    const ID_M: &str = "018f2b1c-4d5e-7abc-8123-000000000003";
    const ID_Z: &str = "018f2b1c-4d5e-7abc-8123-000000000004";

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
            parent: None,
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip_write_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let items = vec![make_item(ID_B, "Second"), make_item(ID_A, "First")];

        write_atomic(&path, &items).unwrap();
        let loaded = read_active(&path).unwrap();

        // Items should be sorted by ID
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, ID_A);
        assert_eq!(loaded[1].id, ID_B);
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

        let item = make_item(ID_A, "Good item");
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

        let item = make_item(ID_A, "Good item");
        let good_line = serde_json::to_string(&item).unwrap();
        let content = format!("{{\"schema_version\":1}}\n{}\n{{bad json\n", good_line);
        fs::write(&path, content).unwrap();

        let items = read_archive(&path).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, ID_A);
    }

    #[test]
    fn items_sorted_by_id_in_output() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let items = vec![
            make_item(ID_Z, "Z"),
            make_item(ID_A, "A"),
            make_item(ID_M, "M"),
        ];

        write_atomic(&path, &items).unwrap();
        let loaded = read_active(&path).unwrap();
        assert_eq!(loaded[0].id, ID_A);
        assert_eq!(loaded[1].id, ID_M);
        assert_eq!(loaded[2].id, ID_Z);
    }

    #[test]
    fn archive_truncated_last_line_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");

        let item = make_item(ID_A, "Good item");
        let good_line = serde_json::to_string(&item).unwrap();
        // Truncated JSON on last line (simulate crash mid-append)
        let content = format!(
            "{{\"schema_version\":1}}\n{}\n{{\"id\":\"{}\",\"tit",
            good_line, ID_B
        );
        fs::write(&path, content).unwrap();

        let items = read_archive(&path).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, ID_A);
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

    #[test]
    fn active_invalid_extension_key_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let item = make_item(ID_A, "Good item");
        let mut json: serde_json::Value = serde_json::to_value(&item).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("bogus".to_string(), serde_json::json!("bad"));
        let line = serde_json::to_string(&json).unwrap();
        let content = format!("{{\"schema_version\":1}}\n{}\n", line);
        fs::write(&path, content).unwrap();

        let result = read_active(&path);
        match result {
            Err(TgError::StorageCorruption(msg)) => {
                assert!(msg.contains("line 2"), "Should mention line 2: {}", msg);
                assert!(
                    msg.contains("must start with 'x-' prefix"),
                    "Should mention x- prefix: {}",
                    msg
                );
            }
            other => panic!("Expected StorageCorruption, got: {:?}", other),
        }
    }

    #[test]
    fn archive_invalid_extension_key_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");

        let good_item = make_item(ID_A, "Good item");
        let good_line = serde_json::to_string(&good_item).unwrap();

        let bad_item = make_item(ID_B, "Bad item");
        let mut bad_json: serde_json::Value = serde_json::to_value(&bad_item).unwrap();
        bad_json
            .as_object_mut()
            .unwrap()
            .insert("bogus".to_string(), serde_json::json!("bad"));
        let bad_line = serde_json::to_string(&bad_json).unwrap();

        let content = format!("{{\"schema_version\":1}}\n{}\n{}\n", good_line, bad_line);
        fs::write(&path, content).unwrap();

        let items = read_archive(&path).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, ID_A);
    }

    #[test]
    fn active_valid_extensions_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tasks.jsonl");

        let mut item = make_item(ID_A, "Item with extensions");
        item.extensions
            .insert("x-custom".to_string(), serde_json::json!("value"));
        item.extensions
            .insert("x-meta".to_string(), serde_json::json!({"key": "val"}));

        write_atomic(&path, &[item]).unwrap();
        let loaded = read_active(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, ID_A);
        assert_eq!(loaded[0].extensions.len(), 2);
    }

    #[test]
    fn write_atomic_rejects_invalid_and_duplicate_ids_without_mutating_file() {
        let cases = [
            vec![make_item("tg-legacy", "Legacy")],
            vec![make_item(ID_A, "First"), make_item(ID_A, "Duplicate")],
        ];

        for items in cases {
            // Arrange
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("tasks.jsonl");
            fs::write(&path, "unchanged\n").unwrap();

            // Act
            let result = write_atomic(&path, &items);

            // Assert
            assert!(result.is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "unchanged\n");
        }
    }

    #[test]
    fn append_to_archive_rejects_invalid_and_duplicate_ids_without_side_effects() {
        // Arrange
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");
        let valid_item = make_item(ID_A, "Archived");
        append_to_archive(&path, &valid_item).unwrap();
        let before_duplicate = fs::read_to_string(&path).unwrap();

        // Act
        let duplicate_result = append_to_archive(&path, &valid_item);
        let invalid_path = tmp.path().join("invalid-archive.jsonl");
        let invalid_result = append_to_archive(&invalid_path, &make_item("tg-legacy", "Legacy"));

        // Assert
        assert!(matches!(
            duplicate_result,
            Err(TgError::StorageCorruption(_))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), before_duplicate);
        assert!(matches!(invalid_result, Err(TgError::InvalidId(_))));
        assert!(!invalid_path.exists());
    }

    #[test]
    fn archive_id_scan_reserves_id_from_item_with_invalid_extensions() {
        // Arrange
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");
        let mut corrupt_item = serde_json::to_value(make_item(ID_A, "Corrupt")).unwrap();
        corrupt_item
            .as_object_mut()
            .unwrap()
            .insert("bogus".to_string(), serde_json::json!(true));
        fs::write(&path, format!("{{\"schema_version\":1}}\n{corrupt_item}\n")).unwrap();

        // Act
        let full_items = read_archive(&path).unwrap();
        let ids = read_archive_ids(&path).unwrap();

        // Assert
        assert!(full_items.is_empty());
        assert_eq!(ids, HashSet::from([ID_A.to_string()]));
    }

    #[test]
    fn archive_id_scan_fails_closed_on_invalid_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.jsonl");
        fs::write(
            &path,
            "{\"schema_version\":1}\n{\"id\":\"tg-legacy\",\"bogus\":true}\n",
        )
        .unwrap();

        let result = read_archive_ids(&path);

        assert!(matches!(result, Err(TgError::StorageCorruption(_))));
    }
}
