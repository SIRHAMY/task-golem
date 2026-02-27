use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};

use chrono::Utc;
use serde::Serialize;

use crate::cli::output;
use task_golem::errors::TgError;
use task_golem::model::deps;
use task_golem::model::item::Item;
use task_golem::store::root;
use task_golem::store::Store;

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

        // Save fixed active store
        store.with_lock(|store| store.save_active(&fixed_active))?;

        if json_mode {
            eprintln!("Fixed {} issues", fixed_count);
        } else {
            output::print_human(&format!("Fixed {} issues. Backups created.", fixed_count));
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
        output::print_human(&format!(
            "Found {} issue(s):",
            report.summary.total
        ));
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
                            message: format!(
                                "{}:{}: invalid schema header",
                                file_name, line_num
                            ),
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
                        message: format!(
                            "{}:{}: malformed item line",
                            file_name, line_num
                        ),
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

fn check_duplicate_ids(
    active_items: &[Item],
    archive_items: &[Item],
    issues: &mut Vec<Issue>,
) {
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

fn check_items_in_both(
    active_items: &[Item],
    archive_items: &[Item],
    issues: &mut Vec<Issue>,
) {
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

fn check_invalid_status(
    path: &std::path::Path,
    file_name: &str,
    issues: &mut Vec<Issue>,
) {
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
