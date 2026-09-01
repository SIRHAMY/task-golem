use std::collections::HashSet;

use uuid::{Uuid, Variant, Version};

use crate::errors::TgError;

const MAX_COLLISION_RETRIES: u32 = 10;

/// Generate a canonical UUIDv7 that does not collide with a known item ID.
pub fn generate_id(existing_ids: &HashSet<String>) -> Result<String, TgError> {
    generate_id_from(existing_ids, || Uuid::now_v7().to_string())
}

fn generate_id_from(
    existing_ids: &HashSet<String>,
    mut next_candidate: impl FnMut() -> String,
) -> Result<String, TgError> {
    for _ in 0..MAX_COLLISION_RETRIES {
        let id = next_candidate();

        if !existing_ids.contains(&id) {
            return Ok(id);
        }
    }

    Err(TgError::IdCollisionExhausted(MAX_COLLISION_RETRIES))
}

/// Validate the canonical lowercase UUIDv7 item ID representation.
pub fn validate_id(input: &str) -> Result<(), TgError> {
    let parsed = Uuid::parse_str(input).map_err(|_| TgError::InvalidId(input.to_string()))?;
    let is_canonical = input.len() == 36 && input == parsed.hyphenated().to_string();
    let is_uuid_v7 = parsed.get_version() == Some(Version::SortRand);
    let is_rfc_variant = parsed.get_variant() == Variant::RFC4122;

    if !is_canonical || !is_uuid_v7 || !is_rfc_variant {
        return Err(TgError::InvalidId(input.to_string()));
    }

    Ok(())
}

/// Validate and resolve an exact item ID in the requested active/archive scope.
pub fn resolve_id(
    input: &str,
    active_ids: &[String],
    archive_ids: &HashSet<String>,
    include_archive: bool,
) -> Result<String, TgError> {
    validate_id(input)?;

    if active_ids.iter().any(|id| id == input) || (include_archive && archive_ids.contains(input)) {
        return Ok(input.to_string());
    }

    Err(TgError::ItemNotFound(input.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE_ID: &str = "018f2b1c-4d5e-7abc-8123-456789abcdef";
    const ARCHIVED_ID: &str = "018f2b1c-4d5e-7abc-9234-56789abcdef0";

    #[test]
    fn generated_ids_are_unique_canonical_uuid_v7s() {
        let existing = HashSet::new();
        let ids: Vec<String> = (0..1_000)
            .map(|_| generate_id(&existing).unwrap())
            .collect();
        let unique_ids: HashSet<&String> = ids.iter().collect();

        assert_eq!(unique_ids.len(), 1_000);
        assert!(ids.iter().all(|id| validate_id(id).is_ok()));
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn validate_id_rejects_noncanonical_and_non_v7_identifiers() {
        let invalid_ids = [
            "tg-a1b2c",
            "tg-018f2b1c-4d5e-7abc-8123-456789abcdef",
            "018f2b1c",
            "018f2b1c4d5e7abc8123456789abcdef",
            "018F2B1C-4D5E-7ABC-8123-456789ABCDEF",
            "550e8400-e29b-41d4-a716-446655440000",
            "018f2b1c-4d5e-7abc-0123-456789abcdef",
            "not-a-uuid",
        ];

        for invalid_id in invalid_ids {
            assert!(
                matches!(validate_id(invalid_id), Err(TgError::InvalidId(id)) if id == invalid_id),
                "expected invalid_id for {invalid_id}"
            );
        }
    }

    #[test]
    fn resolve_id_matches_exact_active_and_archive_ids() {
        let active = vec![ACTIVE_ID.to_string()];
        let archive = HashSet::from([ARCHIVED_ID.to_string()]);

        assert_eq!(
            resolve_id(ACTIVE_ID, &active, &archive, false).unwrap(),
            ACTIVE_ID
        );
        assert_eq!(
            resolve_id(ARCHIVED_ID, &active, &archive, true).unwrap(),
            ARCHIVED_ID
        );
        assert!(matches!(
            resolve_id(ARCHIVED_ID, &active, &archive, false),
            Err(TgError::ItemNotFound(id)) if id == ARCHIVED_ID
        ));
    }

    #[test]
    fn resolve_id_distinguishes_invalid_from_absent() {
        let active = vec![ACTIVE_ID.to_string()];
        let archive = HashSet::new();
        let absent = "018f2b1c-4d5e-7abc-a345-6789abcdef01";

        assert!(matches!(
            resolve_id(absent, &active, &archive, false),
            Err(TgError::ItemNotFound(id)) if id == absent
        ));
        assert!(matches!(
            resolve_id(&ACTIVE_ID.to_uppercase(), &active, &archive, false),
            Err(TgError::InvalidId(_))
        ));
    }

    #[test]
    fn generation_retries_collisions_until_success() {
        let collisions = [ACTIVE_ID.to_string(), ARCHIVED_ID.to_string()];
        let existing = HashSet::from(collisions.clone());
        let available = "018f2b1c-4d5e-7abc-a345-6789abcdef01".to_string();
        let mut candidates = collisions.into_iter().chain([available.clone()]);

        let generated = generate_id_from(&existing, || candidates.next().unwrap()).unwrap();

        assert_eq!(generated, available);
    }

    #[test]
    fn generation_reports_exhausted_collisions() {
        let existing = HashSet::from([ACTIVE_ID.to_string()]);
        let mut attempts = 0;

        let result = generate_id_from(&existing, || {
            attempts += 1;
            ACTIVE_ID.to_string()
        });

        assert!(matches!(
            result,
            Err(TgError::IdCollisionExhausted(MAX_COLLISION_RETRIES))
        ));
        assert_eq!(attempts, MAX_COLLISION_RETRIES);
    }
}
