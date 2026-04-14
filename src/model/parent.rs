//! Single orchestration point for mutating an `Item`'s `parent` field.
//!
//! All paths that change `item.parent` (add, edit, future batch-edit) must go through
//! [`reparent`] so the validate-then-cycle-check-then-mutate invariant cannot be bypassed.

use std::collections::HashSet;

use chrono::Utc;

use crate::errors::TgError;
use crate::model::deps::{validate_parent, would_create_parent_cycle};
use crate::model::item::Item;

/// Apply a reparent mutation in-memory.
///
/// Steps:
/// 1. Locate the source item (`id`) in `items`.
/// 2. If `new_parent` is `Some(pid)`:
///    - Reject self-parent via [`validate_parent`].
///    - Reject dangling/archived targets via [`validate_parent`].
///    - Reject cycles via [`would_create_parent_cycle`] on the proposed graph.
/// 3. Write the new parent and bump `updated_at`.
///
/// `archive` is passed so validation can distinguish "target is archived" from
/// "target does not exist anywhere" (both rejected, but the error text differs).
pub fn reparent(
    items: &mut [Item],
    id: &str,
    new_parent: Option<String>,
    archive: &[Item],
) -> Result<(), TgError> {
    // Confirm the source exists.
    if !items.iter().any(|i| i.id == id) {
        return Err(TgError::ItemNotFound(id.to_string()));
    }

    if let Some(parent_id) = new_parent.as_deref() {
        let active_ids: HashSet<String> = items.iter().map(|i| i.id.clone()).collect();
        let archive_ids: HashSet<String> = archive.iter().map(|i| i.id.clone()).collect();

        validate_parent(id, parent_id, &active_ids, &archive_ids)?;

        if would_create_parent_cycle(items, id, parent_id) {
            return Err(TgError::ParentCycle {
                ids: vec![id.to_string(), parent_id.to_string()],
            });
        }
    }

    let item = items
        .iter_mut()
        .find(|i| i.id == id)
        .expect("existence checked above");
    item.parent = new_parent;
    item.updated_at = Utc::now();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::status::Status;
    use std::collections::BTreeMap;

    fn make_item(id: &str, parent: Option<&str>) -> Item {
        let now = Utc::now();
        Item {
            id: id.to_string(),
            title: format!("Item {}", id),
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
            parent: parent.map(|s| s.to_string()),
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn reparent_sets_parent() {
        let mut items = vec![make_item("tg-aaa00", None), make_item("tg-bbb00", None)];
        reparent(&mut items, "tg-aaa00", Some("tg-bbb00".to_string()), &[]).unwrap();
        let updated = items.iter().find(|i| i.id == "tg-aaa00").unwrap();
        assert_eq!(updated.parent.as_deref(), Some("tg-bbb00"));
    }

    #[test]
    fn reparent_clears_parent() {
        let mut items = vec![
            make_item("tg-aaa00", Some("tg-bbb00")),
            make_item("tg-bbb00", None),
        ];
        reparent(&mut items, "tg-aaa00", None, &[]).unwrap();
        let updated = items.iter().find(|i| i.id == "tg-aaa00").unwrap();
        assert_eq!(updated.parent, None);
    }

    #[test]
    fn self_parent_rejected() {
        let mut items = vec![make_item("tg-aaa00", None)];
        let err = reparent(&mut items, "tg-aaa00", Some("tg-aaa00".to_string()), &[]).unwrap_err();
        assert!(matches!(err, TgError::ParentSelfReference { .. }));
    }

    #[test]
    fn dangling_parent_rejected() {
        let mut items = vec![make_item("tg-aaa00", None)];
        let err = reparent(&mut items, "tg-aaa00", Some("tg-bogus".to_string()), &[]).unwrap_err();
        assert!(matches!(err, TgError::ParentDangling { .. }));
    }

    #[test]
    fn archived_parent_rejected() {
        let mut items = vec![make_item("tg-aaa00", None)];
        let archive = vec![make_item("tg-old00", None)];
        let err = reparent(
            &mut items,
            "tg-aaa00",
            Some("tg-old00".to_string()),
            &archive,
        )
        .unwrap_err();
        assert!(matches!(err, TgError::ParentDangling { .. }));
    }

    #[test]
    fn direct_cycle_rejected() {
        // A's parent is B. Setting B's parent to A would create a cycle A<->B.
        let mut items = vec![
            make_item("tg-aaa00", Some("tg-bbb00")),
            make_item("tg-bbb00", None),
        ];
        let err = reparent(&mut items, "tg-bbb00", Some("tg-aaa00".to_string()), &[]).unwrap_err();
        assert!(matches!(err, TgError::ParentCycle { .. }));
    }

    #[test]
    fn transitive_cycle_rejected() {
        // A -> B -> C. Setting C.parent = A would create A -> B -> C -> A.
        let mut items = vec![
            make_item("tg-aaa00", Some("tg-bbb00")),
            make_item("tg-bbb00", Some("tg-ccc00")),
            make_item("tg-ccc00", None),
        ];
        let err = reparent(&mut items, "tg-ccc00", Some("tg-aaa00".to_string()), &[]).unwrap_err();
        assert!(matches!(err, TgError::ParentCycle { .. }));
    }

    #[test]
    fn unrelated_item_not_found() {
        let mut items = vec![make_item("tg-aaa00", None)];
        let err = reparent(&mut items, "tg-zzz00", None, &[]).unwrap_err();
        assert!(matches!(err, TgError::ItemNotFound(_)));
    }

    #[test]
    fn valid_reparent_to_sibling_works() {
        // A, B, C all top-level. Reparent C under B.
        let mut items = vec![
            make_item("tg-aaa00", None),
            make_item("tg-bbb00", None),
            make_item("tg-ccc00", None),
        ];
        reparent(&mut items, "tg-ccc00", Some("tg-bbb00".to_string()), &[]).unwrap();
        assert_eq!(
            items
                .iter()
                .find(|i| i.id == "tg-ccc00")
                .unwrap()
                .parent
                .as_deref(),
            Some("tg-bbb00")
        );
    }
}
