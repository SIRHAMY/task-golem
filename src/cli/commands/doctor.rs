use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::cli::output;
use task_golem::cache;
use task_golem::errors::TgError;
use task_golem::events::archive as events_archive;
use task_golem::events::record::{CURRENT_EVENT_SCHEMA_VERSION, Event, EventType};
use task_golem::model::deps;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::root;
use task_golem::store::{CACHE_GITIGNORE_LINES, Store};

#[derive(Debug, Serialize)]
struct DoctorReport {
    issues: Vec<Issue>,
    summary: Summary,
}

#[derive(Debug, Serialize)]
struct Summary {
    total: usize,
    by_type: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
struct Issue {
    #[serde(rename = "type")]
    issue_type: String,
    severity: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}

pub fn run(json_mode: bool, fix: bool) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir.clone());

    let mut issues: Vec<Issue> = Vec::new();

    // 1. JSONL syntax check — parse every line in both files
    check_jsonl_syntax(&store.tasks_path(), "tasks.jsonl", true, &mut issues);
    check_jsonl_syntax(&store.archive_path(), "archive.jsonl", false, &mut issues);

    // Load items for remaining checks (use lenient parsing)
    let active_items = load_items_lenient(&store.tasks_path());
    let archive_items = load_items_lenient(&store.archive_path());

    // 2. Duplicate IDs — check across active + archive
    check_duplicate_ids(&active_items, &archive_items, &mut issues);

    // 3. Items in both files
    check_items_in_both(&active_items, &archive_items, &mut issues);

    // 4. Invalid status — already caught by JSONL syntax check since Status is an enum,
    //    but we can double-check the raw lines
    check_invalid_status(&store.tasks_path(), "tasks.jsonl", &mut issues);
    check_invalid_status(&store.archive_path(), "archive.jsonl", &mut issues);

    // 5. Dependency cycles
    check_dependency_cycles(&active_items, &mut issues);

    // 6. Dangling deps
    let active_ids: HashSet<String> = active_items.iter().map(|i| i.id.clone()).collect();
    let archive_ids: HashSet<String> = archive_items.iter().map(|i| i.id.clone()).collect();
    check_dangling_deps(&active_items, &active_ids, &archive_ids, &mut issues);

    // 7. Parent cycles (active items, parent graph only).
    check_parent_cycles(&active_items, &mut issues);

    // 8. Dangling parent references — active side (error, not auto-repaired).
    check_parent_dangling_active(&active_items, &active_ids, &mut issues);

    // 9. Dangling parent references — archive side (warning, auto-repairable).
    check_parent_dangling_archive(&archive_items, &active_ids, &archive_ids, &mut issues);

    // 10. Gitignore hygiene — checked BEFORE cache drift so the drift check's
    //     internal `ensure_gitignore()` side effect doesn't mask a missing
    //     gitignore issue we would have otherwise reported.
    check_gitignore_missing(&store, &mut issues);

    // 11. Cache consistency — rebuild into a temp DB, compare schema_version + row counts.
    //     On drift, repair by atomic-rename (handled in the --fix block below).
    let cache_drift_tmp = check_cache_drift(&store, &mut issues);

    // 12. Events integrity (TG-008 P5):
    //     - events_malformed: JSON lines that don't parse as a v1 Event.
    //     - events_drift_status_mismatch: most-recent status_transition
    //       disagrees with the task's current status.
    //     - events_orphan: events whose task_id is absent from active +
    //       archive + archive-pruned.
    //     - events_in_active_for_archived_task: events for archived tasks
    //       still in events.jsonl (repaired by --fix via move_for_task).
    //     - events_dup_across_active_and_archive: events present in both
    //       files (crash window in move_for_task; repaired by --fix by
    //       dropping dups from active).
    let pruned_ids = load_pruned_ids(&store);
    let active_events = load_events_lenient(&store.events_path());
    let archive_events = load_events_lenient(&store.events_archive_path());
    check_events_malformed(&store.events_path(), "events.jsonl", &mut issues);
    check_events_malformed(
        &store.events_archive_path(),
        "events.archive.jsonl",
        &mut issues,
    );
    check_events_drift(&active_items, &active_events, &mut issues);
    check_events_orphan(
        &active_events,
        &archive_events,
        &active_ids,
        &archive_ids,
        &pruned_ids,
        &mut issues,
    );
    check_events_in_active_for_archived(&active_events, &archive_ids, &mut issues);
    check_events_dup_across_files(&active_events, &archive_events, &mut issues);

    // Apply fixes if requested
    let mut fixed_count = 0;
    if fix && !issues.is_empty() {
        // Create timestamped backups before any repair
        let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let tasks_backup = project_dir.join(format!("tasks.jsonl.bak.{}", timestamp));
        let archive_backup = project_dir.join(format!("archive.jsonl.bak.{}", timestamp));

        if tasks_backup.exists() {
            eprintln!(
                "Warning: backup file already exists: {}",
                tasks_backup.display()
            );
        }
        if archive_backup.exists() {
            eprintln!(
                "Warning: backup file already exists: {}",
                archive_backup.display()
            );
        }

        // Copy current files as backups
        if store.tasks_path().exists() {
            fs::copy(store.tasks_path(), &tasks_backup).map_err(TgError::IoError)?;
        }
        if store.archive_path().exists() {
            fs::copy(store.archive_path(), &archive_backup).map_err(TgError::IoError)?;
        }

        // Fix: remove items that exist in both files (remove from active)
        let both_ids: HashSet<String> = issues
            .iter()
            .filter(|i| i.issue_type == "items_in_both")
            .filter_map(|i| i.details.clone())
            .collect();

        let mut fixed_active: Vec<Item> = active_items
            .iter()
            .filter(|item| !both_ids.contains(&item.id))
            .cloned()
            .collect();

        if !both_ids.is_empty() {
            fixed_count += both_ids.len();
        }

        // Fix: remove dangling deps
        let all_known: HashSet<String> = active_ids.union(&archive_ids).cloned().collect();
        for item in &mut fixed_active {
            let before_len = item.dependencies.len();
            item.dependencies.retain(|dep| all_known.contains(dep));
            if item.dependencies.len() < before_len {
                fixed_count += before_len - item.dependencies.len();
            }
        }

        // Apply all JSONL mutations under a single lock: tasks.jsonl rewrite for
        // items_in_both / dangling deps, and archive.jsonl rewrite for dangling
        // parent refs on archived items. Grouping under one lock avoids a narrow
        // race where a concurrent writer could see a half-repaired store.
        let archive_needs_repair = issues
            .iter()
            .any(|i| i.issue_type == "parent_dangling_archive");
        let archive_all: HashSet<String> = active_ids.union(&archive_ids).cloned().collect();

        let archive_fixes = store.with_lock(|store| {
            store.save_active(&fixed_active)?;

            let mut archive_fixes = 0usize;
            if archive_needs_repair {
                let mut fixed_archive: Vec<Item> = archive_items.clone();
                for item in &mut fixed_archive {
                    if let Some(p) = &item.parent
                        && !archive_all.contains(p)
                    {
                        item.parent = None;
                        archive_fixes += 1;
                    }
                }
                if archive_fixes > 0 {
                    task_golem::store::jsonl::write_atomic(&store.archive_path(), &fixed_archive)?;
                }
            }
            Ok(archive_fixes)
        })?;
        fixed_count += archive_fixes;

        // Fix: cache drift — atomic-rename the prebuilt temp DB over cache.db.
        if let Some(tmp_path) = cache_drift_tmp.as_ref() {
            let cache_path = store.cache_db_path();
            fs::rename(tmp_path, &cache_path).map_err(TgError::IoError)?;
            fixed_count += 1;
        }

        // Fix: missing gitignore lines.
        let gitignore_needs_repair = issues.iter().any(|i| i.issue_type == "gitignore_missing");
        if gitignore_needs_repair {
            store.ensure_gitignore()?;
            fixed_count += 1;
        }

        // Fix: events in active for archived tasks — move to archive via the
        // same helper used by the archive flow. Collect unique task_ids from
        // the relevant issues' details.
        let archived_task_ids_to_move: HashSet<String> = issues
            .iter()
            .filter(|i| i.issue_type == "events_in_active_for_archived_task")
            .filter_map(|i| i.details.clone())
            .collect();
        if !archived_task_ids_to_move.is_empty() {
            let events_count = store.with_lock(|store| {
                let mut count = 0usize;
                for tid in &archived_task_ids_to_move {
                    count += events_archive::move_for_task(
                        &store.events_path(),
                        &store.events_archive_path(),
                        tid,
                    )?;
                }
                Ok::<_, TgError>(count)
            })?;
            fixed_count += events_count;
        }

        // Fix: duplicate events across active + archive — drop the duplicates
        // from active. Rewrite events.jsonl with the deduplicated set. We
        // re-read under lock so we observe any writes from concurrent notes
        // that raced the check, then keep only events not present in
        // archive_events (by the (task_id, ts_utc, text) key).
        let dup_needs_repair = issues
            .iter()
            .any(|i| i.issue_type == "events_dup_across_active_and_archive");
        if dup_needs_repair {
            let dup_count = store.with_lock(|store| {
                let active_evs = load_events_lenient(&store.events_path());
                let archive_evs = load_events_lenient(&store.events_archive_path());
                let archive_keys: HashSet<(String, i64, String)> =
                    archive_evs.iter().map(event_dup_key).collect();
                let (dups, keep): (Vec<Event>, Vec<Event>) = active_evs
                    .into_iter()
                    .partition(|e| archive_keys.contains(&event_dup_key(e)));
                if !dups.is_empty() {
                    rewrite_events_active_atomic(&store.events_path(), &keep)?;
                }
                Ok::<_, TgError>(dups.len())
            })?;
            fixed_count += dup_count;
        }

        if json_mode {
            eprintln!("Fixed {} issues", fixed_count);
        } else {
            output::print_human(&format!("Fixed {} issues. Backups created.", fixed_count));
        }
    } else {
        // Not repairing: clean up the temp rebuild if we created one.
        if let Some(tmp_path) = cache_drift_tmp.as_ref() {
            let _ = fs::remove_file(tmp_path);
        }
    }

    // Build summary
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for issue in &issues {
        *by_type.entry(issue.issue_type.clone()).or_insert(0) += 1;
    }

    let report = DoctorReport {
        summary: Summary {
            total: issues.len(),
            by_type,
        },
        issues,
    };

    if json_mode {
        output::print_json(&report);
    } else if report.summary.total == 0 {
        output::print_human("No issues found. Store is healthy.");
    } else {
        output::print_human(&format!("Found {} issue(s):", report.summary.total));
        for issue in &report.issues {
            output::print_human(&format!(
                "  [{}] {}: {}",
                issue.severity, issue.issue_type, issue.message
            ));
        }
        if !fix {
            output::print_human("\nRun with --fix to attempt automatic repairs.");
        }
    }

    Ok(())
}

fn check_jsonl_syntax(
    path: &std::path::Path,
    file_name: &str,
    strict: bool,
    issues: &mut Vec<Issue>,
) {
    if !path.exists() {
        return;
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            issues.push(Issue {
                issue_type: "jsonl_syntax".to_string(),
                severity: "error".to_string(),
                message: format!("Cannot open {}: {}", file_name, e),
                details: None,
            });
            return;
        }
    };

    let reader = BufReader::new(file);
    for (i, line_result) in reader.lines().enumerate() {
        let line_num = i + 1;
        match line_result {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                if line_num == 1 {
                    // Schema header — just check it's valid JSON
                    if serde_json::from_str::<serde_json::Value>(&line).is_err() {
                        issues.push(Issue {
                            issue_type: "jsonl_syntax".to_string(),
                            severity: "error".to_string(),
                            message: format!("{}:{}: invalid schema header", file_name, line_num),
                            details: None,
                        });
                    }
                    continue;
                }
                // Try to parse as Item
                if serde_json::from_str::<Item>(&line).is_err() {
                    let severity = if strict { "error" } else { "warning" };
                    issues.push(Issue {
                        issue_type: "jsonl_syntax".to_string(),
                        severity: severity.to_string(),
                        message: format!("{}:{}: malformed item line", file_name, line_num),
                        details: None,
                    });
                }
            }
            Err(e) => {
                issues.push(Issue {
                    issue_type: "jsonl_syntax".to_string(),
                    severity: "error".to_string(),
                    message: format!("{}:{}: read error: {}", file_name, line_num, e),
                    details: None,
                });
            }
        }
    }
}

fn load_items_lenient(path: &std::path::Path) -> Vec<Item> {
    if !path.exists() {
        return vec![];
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };

    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for (i, line_result) in reader.lines().enumerate() {
        if i == 0 {
            continue; // Skip schema header
        }
        if let Ok(line) = line_result {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(item) = serde_json::from_str::<Item>(&line) {
                items.push(item);
            }
        }
    }
    items
}

fn check_duplicate_ids(active_items: &[Item], archive_items: &[Item], issues: &mut Vec<Issue>) {
    let mut seen: HashMap<String, Vec<String>> = HashMap::new();

    for item in active_items {
        seen.entry(item.id.clone())
            .or_default()
            .push("tasks.jsonl".to_string());
    }
    for item in archive_items {
        seen.entry(item.id.clone())
            .or_default()
            .push("archive.jsonl".to_string());
    }

    for (id, locations) in &seen {
        if locations.len() > 1 {
            // Check if it's the same file duplicated or across files
            let unique_locations: HashSet<&String> = locations.iter().collect();
            if unique_locations.len() == 1 {
                // Duplicate within same file
                issues.push(Issue {
                    issue_type: "duplicate_id".to_string(),
                    severity: "error".to_string(),
                    message: format!(
                        "Duplicate ID '{}' found {} times in {}",
                        id,
                        locations.len(),
                        locations[0]
                    ),
                    details: Some(id.clone()),
                });
            }
            // Cross-file duplicates are caught by items_in_both check
        }
    }
}

fn check_items_in_both(active_items: &[Item], archive_items: &[Item], issues: &mut Vec<Issue>) {
    let active_ids: HashSet<String> = active_items.iter().map(|i| i.id.clone()).collect();
    let archive_ids: HashSet<String> = archive_items.iter().map(|i| i.id.clone()).collect();

    for id in active_ids.intersection(&archive_ids) {
        issues.push(Issue {
            issue_type: "items_in_both".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "Item '{}' exists in both tasks.jsonl and archive.jsonl (partial tg done failure)",
                id
            ),
            details: Some(id.clone()),
        });
    }
}

fn check_invalid_status(path: &std::path::Path, file_name: &str, issues: &mut Vec<Issue>) {
    if !path.exists() {
        return;
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let reader = BufReader::new(file);
    for (i, line_result) in reader.lines().enumerate() {
        if i == 0 {
            continue; // Skip header
        }
        if let Ok(line) = line_result {
            if line.trim().is_empty() {
                continue;
            }
            // Try to parse as generic JSON and check the status field
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                && let Some(status_str) = value.get("status").and_then(|v| v.as_str())
            {
                let valid = ["todo", "doing", "done", "blocked"];
                if !valid.contains(&status_str) {
                    issues.push(Issue {
                        issue_type: "invalid_status".to_string(),
                        severity: "error".to_string(),
                        message: format!(
                            "{}:{}: invalid status '{}' (valid: todo, doing, done, blocked)",
                            file_name,
                            i + 1,
                            status_str
                        ),
                        details: None,
                    });
                }
            }
        }
    }
}

fn check_dependency_cycles(active_items: &[Item], issues: &mut Vec<Issue>) {
    let cycles = deps::detect_all_cycles(active_items);
    for cycle in &cycles {
        issues.push(Issue {
            issue_type: "dependency_cycle".to_string(),
            severity: "error".to_string(),
            message: format!("Dependency cycle detected: {}", cycle.join(" → ")),
            details: None,
        });
    }
}

fn check_dangling_deps(
    active_items: &[Item],
    active_ids: &HashSet<String>,
    archive_ids: &HashSet<String>,
    issues: &mut Vec<Issue>,
) {
    for item in active_items {
        for dep in &item.dependencies {
            if !active_ids.contains(dep) && !archive_ids.contains(dep) {
                issues.push(Issue {
                    issue_type: "dangling_dep".to_string(),
                    severity: "warning".to_string(),
                    message: format!(
                        "Item '{}' depends on '{}' which is not in active or archive",
                        item.id, dep
                    ),
                    details: Some(dep.clone()),
                });
            }
        }
    }
}

fn check_parent_cycles(active_items: &[Item], issues: &mut Vec<Issue>) {
    let cycles = deps::detect_all_parent_cycles(active_items);
    for cycle in &cycles {
        issues.push(Issue {
            issue_type: "parent_cycle".to_string(),
            severity: "error".to_string(),
            message: format!("Parent cycle detected: {}", cycle.join(" → ")),
            details: Some(cycle.join(",")),
        });
    }
}

fn check_parent_dangling_active(
    active_items: &[Item],
    active_ids: &HashSet<String>,
    issues: &mut Vec<Issue>,
) {
    for item in active_items {
        if let Some(parent) = &item.parent
            && !active_ids.contains(parent)
        {
            issues.push(Issue {
                issue_type: "parent_dangling_active".to_string(),
                severity: "error".to_string(),
                message: format!(
                    "Item '{}' has parent '{}' which is not an active task",
                    item.id, parent
                ),
                details: Some(parent.clone()),
            });
        }
    }
}

fn check_parent_dangling_archive(
    archive_items: &[Item],
    active_ids: &HashSet<String>,
    archive_ids: &HashSet<String>,
    issues: &mut Vec<Issue>,
) {
    for item in archive_items {
        if let Some(parent) = &item.parent
            && !active_ids.contains(parent)
            && !archive_ids.contains(parent)
        {
            issues.push(Issue {
                issue_type: "parent_dangling_archive".to_string(),
                severity: "warning".to_string(),
                message: format!(
                    "Archived item '{}' has parent '{}' which is not in active or archive",
                    item.id, parent
                ),
                details: Some(parent.clone()),
            });
        }
    }
}

/// Check whether the on-disk cache disagrees with a fresh rebuild.
///
/// Builds a fresh temp DB (`cache.db.drift-<pid>`), opens both it and the
/// existing `cache.db`, and compares `schema_version` + per-table row counts.
/// If they agree, the temp file is removed and no issue is emitted. If they
/// disagree (or the existing cache is missing/unreadable), a `cache_drift`
/// issue is emitted and the temp path is returned so the `--fix` block can
/// atomic-rename it over `cache.db` for the repair.
///
/// If the rebuild itself fails, the temp is cleaned up and no issue is emitted
/// (rebuild failures will surface elsewhere — e.g. on the next `tg query`).
fn check_cache_drift(store: &Store, issues: &mut Vec<Issue>) -> Option<std::path::PathBuf> {
    let cache_path = store.cache_db_path();
    // A missing cache is normal — it's built lazily on first query, and the
    // next `tg query` (or `tg doctor --fix` via a separate path) will create
    // it. Don't flag "missing" as drift; only flag disagreement between an
    // existing cache and the authoritative JSONL.
    if !cache_path.exists() {
        return None;
    }

    let project_dir = store.project_dir();
    let tmp_path = project_dir.join(format!("cache.db.drift-{}", process::id()));
    // Clean any leftover from a previous crashed doctor run with same PID.
    let _ = fs::remove_file(&tmp_path);

    if let Err(e) = cache::rebuild::rebuild_to(store, &tmp_path) {
        issues.push(Issue {
            issue_type: "cache_rebuild_failed".to_string(),
            severity: "error".to_string(),
            message: format!("Fresh cache rebuild failed during consistency check: {}", e),
            details: None,
        });
        let _ = fs::remove_file(&tmp_path);
        return None;
    }

    let drift_detail = compare_cache_dbs(&cache_path, &tmp_path);

    match drift_detail {
        CacheComparison::Identical => {
            let _ = fs::remove_file(&tmp_path);
            None
        }
        CacheComparison::Differs(detail) => {
            issues.push(Issue {
                issue_type: "cache_drift".to_string(),
                severity: "warning".to_string(),
                message: format!("Cache disagrees with fresh rebuild: {}", detail),
                details: None,
            });
            Some(tmp_path)
        }
    }
}

enum CacheComparison {
    Identical,
    Differs(String),
}

/// Compare two cache DBs by schema_version + per-table row counts. Any read
/// failure on `existing` counts as drift (the existing cache is broken; repair
/// installs the fresh rebuild). `fresh` read failures bubble up as drift too
/// — they shouldn't happen since we just built it, but we don't want to panic.
fn compare_cache_dbs(existing: &Path, fresh: &Path) -> CacheComparison {
    if !existing.exists() {
        return CacheComparison::Differs("cache.db missing".to_string());
    }

    let existing_conn = match rusqlite::Connection::open_with_flags(
        existing,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => return CacheComparison::Differs(format!("cannot open existing cache: {}", e)),
    };

    let fresh_conn = match rusqlite::Connection::open_with_flags(
        fresh,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => return CacheComparison::Differs(format!("cannot open fresh cache: {}", e)),
    };

    // Schema version from _cache_meta.
    let exist_ver = read_schema_version(&existing_conn);
    let fresh_ver = read_schema_version(&fresh_conn);
    if exist_ver != fresh_ver {
        return CacheComparison::Differs(format!(
            "schema_version {} vs {}",
            exist_ver
                .map(|v| v.to_string())
                .unwrap_or_else(|| "<none>".into()),
            fresh_ver
                .map(|v| v.to_string())
                .unwrap_or_else(|| "<none>".into()),
        ));
    }

    // Per-table row count comparison.
    for table in &["tasks", "task_tags", "task_deps", "task_view"] {
        let exist_count = count_rows(&existing_conn, table);
        let fresh_count = count_rows(&fresh_conn, table);
        match (exist_count, fresh_count) {
            (Some(a), Some(b)) if a != b => {
                return CacheComparison::Differs(format!(
                    "row count in {} ({} vs {})",
                    table, a, b
                ));
            }
            (None, _) | (_, None) => {
                return CacheComparison::Differs(format!("cannot count rows in {}", table));
            }
            _ => {}
        }
    }

    CacheComparison::Identical
}

fn read_schema_version(conn: &rusqlite::Connection) -> Option<u32> {
    conn.query_row(
        "SELECT value FROM _cache_meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| s.parse::<u32>().ok())
}

fn count_rows(conn: &rusqlite::Connection, table: &str) -> Option<i64> {
    // Table name is from a static allowlist above — safe to interpolate.
    let sql = format!("SELECT COUNT(*) FROM {}", table);
    conn.query_row(&sql, [], |row| row.get::<_, i64>(0)).ok()
}

// ----- Events integrity checks (TG-008 P5) -----

/// Lenient event loader used by doctor. Mirrors `events::read::all` but does
/// not emit stderr warnings (the `events_malformed` check is the surfacing
/// mechanism — we don't want noise on every doctor pass).
fn load_events_lenient(path: &std::path::Path) -> Vec<Event> {
    if !path.exists() {
        return vec![];
    }
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = BufReader::new(file);
    let mut events: Vec<Event> = Vec::new();
    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        // Prelude-first: skip unknown schema versions silently to match the
        // library reader's forward-compat posture.
        if let Ok(prelude) = serde_json::from_str::<serde_json::Value>(&line)
            && let Some(v) = prelude.get("v").and_then(|v| v.as_u64())
            && v as u32 != CURRENT_EVENT_SCHEMA_VERSION
        {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Event>(&line) {
            events.push(event);
        }
    }
    events
}

/// Load task IDs from `archive-pruned.jsonl`, if it exists. Used by the orphan
/// check to avoid flagging events for tasks that were intentionally pruned.
/// Lenient: skips the schema header (parses as a non-Item) and any line that
/// doesn't round-trip as an `Item`.
fn load_pruned_ids(store: &Store) -> HashSet<String> {
    let path = store.project_dir().join("archive-pruned.jsonl");
    if !path.exists() {
        return HashSet::new();
    }
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return HashSet::new(),
    };
    let reader = BufReader::new(file);
    let mut ids = HashSet::new();
    for line_result in reader.lines() {
        if let Ok(line) = line_result
            && !line.trim().is_empty()
            && let Ok(item) = serde_json::from_str::<Item>(&line)
        {
            ids.insert(item.id);
        }
    }
    ids
}

/// Check for malformed event lines. Emits one issue per malformed line with
/// the file name + 1-based line number in `details`.
fn check_events_malformed(path: &std::path::Path, file_name: &str, issues: &mut Vec<Issue>) {
    if !path.exists() {
        return;
    }
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            issues.push(Issue {
                issue_type: "events_malformed".to_string(),
                severity: "error".to_string(),
                message: format!("Cannot open {}: {}", file_name, e),
                details: None,
            });
            return;
        }
    };
    let reader = BufReader::new(file);
    for (i, line_result) in reader.lines().enumerate() {
        let line_num = i + 1;
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                issues.push(Issue {
                    issue_type: "events_malformed".to_string(),
                    severity: "error".to_string(),
                    message: format!("{}:{}: read error: {}", file_name, line_num, e),
                    details: Some(format!("{}:{}", file_name, line_num)),
                });
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        // Peek at `v` first: unknown versions are forward-compat (not
        // malformed). A line that won't parse as JSON at all is malformed.
        let prelude: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                issues.push(Issue {
                    issue_type: "events_malformed".to_string(),
                    severity: "error".to_string(),
                    message: format!("{}:{}: invalid JSON: {}", file_name, line_num, e),
                    details: Some(format!("{}:{}", file_name, line_num)),
                });
                continue;
            }
        };
        let v = prelude.get("v").and_then(|v| v.as_u64()).map(|v| v as u32);
        if v != Some(CURRENT_EVENT_SCHEMA_VERSION) {
            // Unknown version: skip silently (forward-compat).
            continue;
        }
        if let Err(e) = serde_json::from_str::<Event>(&line) {
            issues.push(Issue {
                issue_type: "events_malformed".to_string(),
                severity: "error".to_string(),
                message: format!("{}:{}: malformed event: {}", file_name, line_num, e),
                details: Some(format!("{}:{}", file_name, line_num)),
            });
        }
    }
}

/// Check that each active task's most-recent `status_transition` event's
/// target status matches the task's current status. Skips tasks with no
/// events (pre-TG-008 tasks or never-transitioned tasks).
fn check_events_drift(active_items: &[Item], active_events: &[Event], issues: &mut Vec<Issue>) {
    // Index most-recent status_transition per task_id.
    let mut latest: HashMap<String, (DateTime<Utc>, Status)> = HashMap::new();
    for event in active_events {
        if event.event_type != EventType::StatusTransition {
            continue;
        }
        let Some(status) = event.status else {
            continue;
        };
        latest
            .entry(event.task_id.clone())
            .and_modify(|(ts, st)| {
                if event.ts > *ts {
                    *ts = event.ts;
                    *st = status;
                }
            })
            .or_insert((event.ts, status));
    }
    for item in active_items {
        if let Some((ts, expected_status)) = latest.get(&item.id)
            && *expected_status != item.status
        {
            issues.push(Issue {
                issue_type: "events_drift_status_mismatch".to_string(),
                severity: "warning".to_string(),
                message: format!(
                    "Task '{}' status '{}' disagrees with most-recent status_transition event '{}' at {}",
                    item.id, item.status, expected_status, ts
                ),
                details: Some(item.id.clone()),
            });
        }
    }
}

/// Flag events whose `task_id` is absent from active + archive + pruned.
fn check_events_orphan(
    active_events: &[Event],
    archive_events: &[Event],
    active_ids: &HashSet<String>,
    archive_ids: &HashSet<String>,
    pruned_ids: &HashSet<String>,
    issues: &mut Vec<Issue>,
) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in active_events.iter().chain(archive_events.iter()) {
        if !active_ids.contains(&e.task_id)
            && !archive_ids.contains(&e.task_id)
            && !pruned_ids.contains(&e.task_id)
        {
            *counts.entry(e.task_id.clone()).or_insert(0) += 1;
        }
    }
    for (task_id, count) in counts {
        issues.push(Issue {
            issue_type: "events_orphan".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "Orphan events for task_id '{}' ({} event(s); task not in active, archive, or pruned)",
                task_id, count
            ),
            details: Some(task_id),
        });
    }
}

/// Flag events present in `events.jsonl` whose `task_id` is in `archive.jsonl`.
fn check_events_in_active_for_archived(
    active_events: &[Event],
    archive_task_ids: &HashSet<String>,
    issues: &mut Vec<Issue>,
) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in active_events {
        if archive_task_ids.contains(&e.task_id) {
            *counts.entry(e.task_id.clone()).or_insert(0) += 1;
        }
    }
    for (task_id, count) in counts {
        issues.push(Issue {
            issue_type: "events_in_active_for_archived_task".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "{} event(s) in events.jsonl for archived task '{}' — will be moved on --fix",
                count, task_id
            ),
            details: Some(task_id),
        });
    }
}

/// Flag events that appear in both `events.jsonl` and `events.archive.jsonl`
/// (key: `(task_id, ts, text)`). Indicates an interrupted `move_for_task`.
fn check_events_dup_across_files(
    active_events: &[Event],
    archive_events: &[Event],
    issues: &mut Vec<Issue>,
) {
    let archive_keys: HashSet<(String, i64, String)> =
        archive_events.iter().map(event_dup_key).collect();
    let mut dup_task_ids: HashMap<String, usize> = HashMap::new();
    for e in active_events {
        let key = event_dup_key(e);
        if archive_keys.contains(&key) {
            *dup_task_ids.entry(e.task_id.clone()).or_insert(0) += 1;
        }
    }
    for (task_id, count) in dup_task_ids {
        issues.push(Issue {
            issue_type: "events_dup_across_active_and_archive".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "{} event(s) for task '{}' appear in both events.jsonl and events.archive.jsonl — will be removed from active on --fix",
                count, task_id
            ),
            details: Some(task_id),
        });
    }
}

/// Key used to detect duplicate events across active/archive. Uses microsecond
/// precision timestamps (matching the on-disk format) so two events stamped
/// the same instant but with different content are treated as distinct.
fn event_dup_key(e: &Event) -> (String, i64, String) {
    (e.task_id.clone(), e.ts.timestamp_micros(), e.text.clone())
}

/// Atomic rewrite of `events.jsonl` with the given events. Used only by the
/// dup-repair fix path. Inline here (rather than reusing the headerless
/// rewriter inside `events::archive`) to keep the cross-module surface
/// narrow. Same durability contract: tempfile in same directory, fsync,
/// rename.
fn rewrite_events_active_atomic(path: &std::path::Path, events: &[Event]) -> Result<(), TgError> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| {
        TgError::IoError(std::io::Error::other(
            "cannot determine parent dir for events.jsonl rewrite",
        ))
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(TgError::IoError)?;
    for event in events {
        let line = serde_json::to_string(event)
            .expect("Event serialization cannot fail: all fields are safely typed");
        tmp.write_all(line.as_bytes()).map_err(TgError::IoError)?;
        tmp.write_all(b"\n").map_err(TgError::IoError)?;
    }
    tmp.as_file().sync_all().map_err(TgError::IoError)?;
    tmp.persist(path).map_err(|e| TgError::IoError(e.error))?;
    Ok(())
}

fn check_gitignore_missing(store: &Store, issues: &mut Vec<Issue>) {
    let path = store.gitignore_path();
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => {
            // Unreadable (permissions, etc.) — flag as missing so --fix re-creates.
            String::new()
        }
    };

    let present: HashSet<&str> = existing
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let missing: Vec<&&str> = CACHE_GITIGNORE_LINES
        .iter()
        .filter(|line| !present.contains(**line))
        .collect();

    if !missing.is_empty() {
        let missing_display: Vec<String> = missing.iter().map(|s| (**s).to_string()).collect();
        issues.push(Issue {
            issue_type: "gitignore_missing".to_string(),
            severity: "warning".to_string(),
            message: format!(
                ".task-golem/.gitignore missing cache entries: {}",
                missing_display.join(", ")
            ),
            details: Some(missing_display.join(",")),
        });
    }
}
