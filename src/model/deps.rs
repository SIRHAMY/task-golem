use std::collections::{HashMap, HashSet};

use crate::errors::TgError;
use crate::model::item::Item;

/// Check if adding `new_dep_id` as a dependency of `source_id` would create a cycle.
/// Uses DFS from new_dep_id following dependency edges in active items only.
pub fn would_create_cycle(items: &[Item], source_id: &str, new_dep_id: &str) -> bool {
    // Build adjacency: item -> its dependencies
    let dep_map: HashMap<&str, &[String]> = items
        .iter()
        .map(|item| (item.id.as_str(), item.dependencies.as_slice()))
        .collect();

    // DFS from new_dep_id: if we can reach source_id, adding the edge creates a cycle
    let mut visited = HashSet::new();
    let mut stack = vec![new_dep_id];

    while let Some(current) = stack.pop() {
        if current == source_id {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(deps) = dep_map.get(current) {
            for dep in *deps {
                stack.push(dep.as_str());
            }
        }
    }

    false
}

/// Validate a dependency ID. Returns warnings for informational issues.
/// - Rejects self-dependencies
/// - Checks existence in active or archive
/// - Warns on missing from both stores
pub fn validate_dep(
    source_id: &str,
    dep_id: &str,
    active_ids: &HashSet<String>,
    archive_ids: &HashSet<String>,
) -> Result<Vec<String>, TgError> {
    if source_id == dep_id {
        return Err(TgError::InvalidInput(format!(
            "Item cannot depend on itself: {}",
            dep_id
        )));
    }

    let mut warnings = Vec::new();
    if !active_ids.contains(dep_id) && !archive_ids.contains(dep_id) {
        warnings.push(format!(
            "Warning: dependency '{}' not found in active or archive",
            dep_id
        ));
    }

    Ok(warnings)
}

/// Full-graph cycle detection via topological sort (Kahn's algorithm).
/// Returns all cycles found as vectors of item IDs.
/// For use by `tg doctor` in Phase 4.
#[allow(dead_code)] // Used in Phase 4
pub fn detect_all_cycles(items: &[Item]) -> Vec<Vec<String>> {
    let ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();

    // Build in-degree map and adjacency (only for active items)
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for item in items {
        in_degree.entry(item.id.as_str()).or_insert(0);
        for dep in &item.dependencies {
            if ids.contains(dep.as_str()) {
                *in_degree.entry(item.id.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(item.id.as_str());
            }
        }
    }

    // Kahn's: start with nodes that have in-degree 0
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| *id)
        .collect();

    let mut processed = HashSet::new();
    while let Some(node) = queue.pop() {
        processed.insert(node);
        if let Some(deps) = dependents.get(node) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(dep);
                    }
                }
            }
        }
    }

    // Remaining nodes are in cycles
    let cycle_nodes: HashSet<&str> = ids.difference(&processed).copied().collect();
    if cycle_nodes.is_empty() {
        return vec![];
    }

    // Extract individual cycles via DFS
    let mut found_cycles = Vec::new();
    let mut globally_visited = HashSet::new();

    for &start in &cycle_nodes {
        if globally_visited.contains(start) {
            continue;
        }

        let mut path: Vec<&str> = Vec::new();
        let mut path_set: HashSet<&str> = HashSet::new();
        let mut stack: Vec<(&str, bool)> = vec![(start, false)];

        while let Some((node, backtrack)) = stack.pop() {
            if backtrack {
                path_set.remove(node);
                path.pop();
                continue;
            }

            if path_set.contains(node) {
                // Found a cycle - extract it
                let cycle_start = path.iter().position(|&n| n == node).unwrap();
                let cycle: Vec<String> =
                    path[cycle_start..].iter().map(|s| s.to_string()).collect();
                found_cycles.push(cycle);
                continue;
            }

            if !cycle_nodes.contains(node) {
                continue;
            }

            path.push(node);
            path_set.insert(node);
            globally_visited.insert(node);

            stack.push((node, true)); // backtrack marker

            let item = items.iter().find(|i| i.id == node);
            if let Some(item) = item {
                for dep in &item.dependencies {
                    if cycle_nodes.contains(dep.as_str()) {
                        stack.push((dep.as_str(), false));
                    }
                }
            }
        }
    }

    found_cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::BTreeMap;

    use crate::model::status::Status;

    fn make_item(id: &str, deps: Vec<&str>) -> Item {
        let now = Utc::now();
        Item {
            id: id.to_string(),
            title: format!("Item {}", id),
            status: Status::Todo,
            priority: 0,
            description: None,
            tags: vec![],
            dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
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
    fn self_dep_rejected() {
        let active_ids: HashSet<String> = ["tg-aaa00".to_string()].into();
        let archive_ids = HashSet::new();
        let result = validate_dep("tg-aaa00", "tg-aaa00", &active_ids, &archive_ids);
        assert!(result.is_err());
    }

    #[test]
    fn direct_cycle_detected() {
        let items = vec![
            make_item("tg-aaa00", vec!["tg-bbb00"]),
            make_item("tg-bbb00", vec![]),
        ];
        // Adding B->A would create cycle: A->B->A
        assert!(would_create_cycle(&items, "tg-bbb00", "tg-aaa00"));
    }

    #[test]
    fn transitive_cycle_detected() {
        let items = vec![
            make_item("tg-aaa00", vec!["tg-bbb00"]),
            make_item("tg-bbb00", vec!["tg-ccc00"]),
            make_item("tg-ccc00", vec![]),
        ];
        // Adding C->A would create cycle: A->B->C->A
        assert!(would_create_cycle(&items, "tg-ccc00", "tg-aaa00"));
    }

    #[test]
    fn diamond_not_cyclic() {
        let items = vec![
            make_item("tg-aaa00", vec!["tg-bbb00", "tg-ccc00"]),
            make_item("tg-bbb00", vec!["tg-ddd00"]),
            make_item("tg-ccc00", vec!["tg-ddd00"]),
            make_item("tg-ddd00", vec![]),
        ];
        // Adding D->B is not a cycle (D has no path to B through deps of B or its children)
        // Wait - D has no deps, and B depends on D. Adding D->A:
        // A->B->D and A->C->D. If we add D->A that creates a cycle.
        assert!(would_create_cycle(&items, "tg-ddd00", "tg-aaa00"));
        // But adding a new item E that depends on B and C is fine
        assert!(!would_create_cycle(&items, "tg-aaa00", "tg-ddd00"));
        // A depends on D already via B, but adding direct dep doesn't create a cycle
    }

    #[test]
    fn dep_on_archived_item_no_warning() {
        let active_ids: HashSet<String> = ["tg-aaa00".to_string()].into();
        let archive_ids: HashSet<String> = ["tg-bbb00".to_string()].into();
        let warnings = validate_dep("tg-aaa00", "tg-bbb00", &active_ids, &archive_ids).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn dep_on_nonexistent_warns() {
        let active_ids: HashSet<String> = ["tg-aaa00".to_string()].into();
        let archive_ids = HashSet::new();
        let warnings =
            validate_dep("tg-aaa00", "tg-zzz00", &active_ids, &archive_ids).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("not found"));
    }

    #[test]
    fn detect_all_cycles_none() {
        let items = vec![
            make_item("tg-aaa00", vec!["tg-bbb00"]),
            make_item("tg-bbb00", vec![]),
        ];
        assert!(detect_all_cycles(&items).is_empty());
    }

    #[test]
    fn detect_all_cycles_direct() {
        let items = vec![
            make_item("tg-aaa00", vec!["tg-bbb00"]),
            make_item("tg-bbb00", vec!["tg-aaa00"]),
        ];
        let cycles = detect_all_cycles(&items);
        assert!(!cycles.is_empty());
    }

    #[test]
    fn multiple_deps_checked_correctly() {
        let items = vec![
            make_item("tg-aaa00", vec![]),
            make_item("tg-bbb00", vec![]),
            make_item("tg-ccc00", vec!["tg-aaa00"]),
        ];
        // Adding dep from C to B is fine
        assert!(!would_create_cycle(&items, "tg-ccc00", "tg-bbb00"));
        // Adding dep from A to C would create cycle: C->A->C? No, C depends on A, adding A->C creates A->C->A
        assert!(would_create_cycle(&items, "tg-aaa00", "tg-ccc00"));
    }
}
