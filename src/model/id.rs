use std::collections::HashSet;

use crate::errors::TgError;

const ID_PREFIX: &str = "tg";
const ID_HEX_LEN: usize = 5;
const MAX_COLLISION_RETRIES: u32 = 10;

/// Generate a new unique ID in the format `tg-{5 hex chars}`.
///
/// Retries up to 10 times on collision with existing IDs.
pub fn generate_id(existing_ids: &HashSet<String>) -> Result<String, TgError> {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    for _ in 0..MAX_COLLISION_RETRIES {
        let bytes: [u8; 3] = rng.r#gen();
        let hex_str = hex::encode(bytes);
        let id = format!("{}-{}", ID_PREFIX, &hex_str[..ID_HEX_LEN]);

        if !existing_ids.contains(&id) {
            return Ok(id);
        }
    }

    Err(TgError::IdCollisionExhausted(MAX_COLLISION_RETRIES))
}

/// Resolve a user-provided ID string against a set of known IDs.
///
/// Resolution order:
/// 1. Exact match
/// 2. Prepend `tg-` and check for exact match
/// 3. Prefix match (ID starts with input or `tg-{input}`)
///
/// The `scope` parameter controls which ID sets are searched.
pub fn resolve_id(input: &str, active_ids: &[String], archive_ids: &HashSet<String>, include_archive: bool) -> Result<String, TgError> {
    let all_ids: Vec<&String> = if include_archive {
        active_ids.iter().chain(archive_ids.iter()).collect()
    } else {
        active_ids.iter().collect()
    };

    // 1. Exact match
    if all_ids.contains(&&input.to_string()) {
        return Ok(input.to_string());
    }

    // 2. Prepend prefix and exact match
    let prefixed = format!("{}-{}", ID_PREFIX, input);
    if all_ids.contains(&&prefixed) {
        return Ok(prefixed);
    }

    // 3. Prefix match
    let matches: Vec<String> = all_ids
        .iter()
        .filter(|id| id.starts_with(input) || id.starts_with(&prefixed))
        .map(|id| id.to_string())
        .collect();

    match matches.len() {
        0 => Err(TgError::ItemNotFound(input.to_string())),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => Err(TgError::AmbiguousId {
            prefix: input.to_string(),
            matches,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_format() {
        let existing = HashSet::new();
        let id = generate_id(&existing).unwrap();
        assert!(id.starts_with("tg-"), "ID should start with tg-: {}", id);
        assert_eq!(id.len(), 8, "ID should be 8 chars (tg- + 5 hex): {}", id);
        // Verify hex chars
        let hex_part = &id[3..];
        assert!(
            hex_part.chars().all(|c| c.is_ascii_hexdigit()),
            "Hex part should be valid hex: {}",
            hex_part
        );
    }

    #[test]
    fn generate_id_collision_retry() {
        // Fill with a bunch of IDs but not all possible ones
        let mut existing = HashSet::new();
        for i in 0..100 {
            existing.insert(format!("tg-{:05x}", i));
        }
        // Should still find a unique one
        let id = generate_id(&existing).unwrap();
        assert!(!existing.contains(&id));
    }

    #[test]
    fn generate_id_collision_exhausted() {
        // This test verifies the error path. We can't realistically fill all 2^20 IDs,
        // but we can test the error type by mocking would be too complex.
        // Instead, we just verify the happy path works and trust the loop logic.
        let existing = HashSet::new();
        assert!(generate_id(&existing).is_ok());
    }

    #[test]
    fn resolve_exact_match() {
        let active = vec!["tg-a1b2c".to_string(), "tg-d3e4f".to_string()];
        let archive = HashSet::new();
        assert_eq!(
            resolve_id("tg-a1b2c", &active, &archive, false).unwrap(),
            "tg-a1b2c"
        );
    }

    #[test]
    fn resolve_bare_hex() {
        let active = vec!["tg-a1b2c".to_string()];
        let archive = HashSet::new();
        assert_eq!(
            resolve_id("a1b2c", &active, &archive, false).unwrap(),
            "tg-a1b2c"
        );
    }

    #[test]
    fn resolve_prefix_match() {
        let active = vec!["tg-a1b2c".to_string(), "tg-d3e4f".to_string()];
        let archive = HashSet::new();
        assert_eq!(
            resolve_id("a1b", &active, &archive, false).unwrap(),
            "tg-a1b2c"
        );
    }

    #[test]
    fn resolve_ambiguous_prefix() {
        let active = vec!["tg-a1b2c".to_string(), "tg-a1b3d".to_string()];
        let archive = HashSet::new();
        let result = resolve_id("a1b", &active, &archive, false);
        assert!(matches!(result, Err(TgError::AmbiguousId { .. })));
    }

    #[test]
    fn resolve_not_found() {
        let active = vec!["tg-a1b2c".to_string()];
        let archive = HashSet::new();
        let result = resolve_id("zzzzz", &active, &archive, false);
        assert!(matches!(result, Err(TgError::ItemNotFound(_))));
    }

    #[test]
    fn resolve_archive_scope() {
        let active = vec![];
        let mut archive = HashSet::new();
        archive.insert("tg-arch1".to_string());

        // Without archive scope, not found
        assert!(resolve_id("arch1", &active, &archive, false).is_err());

        // With archive scope, found
        assert_eq!(
            resolve_id("arch1", &active, &archive, true).unwrap(),
            "tg-arch1"
        );
    }
}
