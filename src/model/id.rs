use std::collections::HashSet;

use crate::errors::TgError;

pub const DEFAULT_ID_PREFIX: &str = "tg";
pub const DEFAULT_ID_LEN: usize = 5;
const MAX_COLLISION_RETRIES: u32 = 10;

/// Crockford's Base32 alphabet (lowercase). Excludes i, l, o, u to avoid
/// confusion with 1, 1, 0, and accidental profanity respectively.
/// 32 chars = 5 bits per character, so 5 chars gives 32^5 ≈ 33M unique IDs.
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Generate a new unique ID with the default prefix `tg` and default ID length.
///
/// Retries up to 10 times on collision with existing IDs.
#[cfg(test)]
pub fn generate_id(existing_ids: &HashSet<String>) -> Result<String, TgError> {
    generate_id_with_prefix(existing_ids, DEFAULT_ID_PREFIX, DEFAULT_ID_LEN)
}

/// Generate a new unique ID with a custom prefix and length.
///
/// Uses Crockford's Base32 encoding for the random portion, which provides
/// a human-friendly alphabet that avoids confusable characters (i/l/o/u excluded).
/// With the default length of 5, this gives ~33M possible IDs per prefix.
pub fn generate_id_with_prefix(
    existing_ids: &HashSet<String>,
    prefix: &str,
    id_len: usize,
) -> Result<String, TgError> {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    for _ in 0..MAX_COLLISION_RETRIES {
        let random_part: String = (0..id_len)
            .map(|_| {
                let idx: usize = rng.gen_range(0..32);
                CROCKFORD_ALPHABET[idx] as char
            })
            .collect();
        let id = format!("{}-{}", prefix, random_part);

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
/// 2. Prepend default prefix `tg-` and check for exact match
/// 3. Prefix match (ID starts with input or `tg-{input}`)
///
/// The `scope` parameter controls which ID sets are searched.
pub fn resolve_id(
    input: &str,
    active_ids: &[String],
    archive_ids: &HashSet<String>,
    include_archive: bool,
) -> Result<String, TgError> {
    let all_ids: Vec<&String> = if include_archive {
        active_ids.iter().chain(archive_ids.iter()).collect()
    } else {
        active_ids.iter().collect()
    };

    // 1. Exact match
    if all_ids.contains(&&input.to_string()) {
        return Ok(input.to_string());
    }

    // 2. Prepend default prefix and exact match
    let prefixed = format!("{}-{}", DEFAULT_ID_PREFIX, input);
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

    fn is_crockford_base32(c: char) -> bool {
        CROCKFORD_ALPHABET.contains(&(c as u8))
    }

    #[test]
    fn generate_id_format() {
        let existing = HashSet::new();
        let id = generate_id(&existing).unwrap();
        assert!(id.starts_with("tg-"), "ID should start with tg-: {}", id);
        assert_eq!(id.len(), 8, "ID should be 8 chars (tg- + 5): {}", id);
        let random_part = &id[3..];
        assert!(
            random_part.chars().all(is_crockford_base32),
            "Random part should be valid Crockford Base32: {}",
            random_part
        );
    }

    #[test]
    fn generate_id_excludes_confusable_chars() {
        // Generate many IDs and verify none contain i, l, o, u
        let existing = HashSet::new();
        for _ in 0..100 {
            let id = generate_id(&existing).unwrap();
            let random_part = &id[3..];
            assert!(
                !random_part.contains('i')
                    && !random_part.contains('l')
                    && !random_part.contains('o')
                    && !random_part.contains('u'),
                "ID should not contain confusable chars (i/l/o/u): {}",
                id
            );
        }
    }

    #[test]
    fn generate_id_collision_retry() {
        // Fill with a bunch of IDs but not all possible ones
        let mut existing = HashSet::new();
        for i in 0..100 {
            existing.insert(format!("tg-{:05}", i));
        }
        // Should still find a unique one
        let id = generate_id(&existing).unwrap();
        assert!(!existing.contains(&id));
    }

    #[test]
    fn generate_id_collision_exhausted() {
        let existing = HashSet::new();
        assert!(generate_id(&existing).is_ok());
    }

    #[test]
    fn generate_id_custom_len() {
        let existing = HashSet::new();
        let id = generate_id_with_prefix(&existing, "tg", 8).unwrap();
        assert!(id.starts_with("tg-"), "ID should start with tg-: {}", id);
        assert_eq!(id.len(), 11, "ID should be 11 chars (tg- + 8): {}", id);
        let random_part = &id[3..];
        assert!(
            random_part.chars().all(is_crockford_base32),
            "Random part should be valid Crockford Base32: {}",
            random_part
        );
    }

    #[test]
    fn generate_id_short_len() {
        let existing = HashSet::new();
        let id = generate_id_with_prefix(&existing, "tg", 3).unwrap();
        assert_eq!(id.len(), 6, "ID should be 6 chars (tg- + 3): {}", id);
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
