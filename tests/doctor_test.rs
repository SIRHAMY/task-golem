mod common;

use std::fs;

use common::TestProject;

#[test]
fn doctor_clean_store_no_issues() {
    let project = TestProject::new().unwrap();

    // Add some items to have a non-empty store
    project.run_tg(&["add", "Task A"]);
    project.run_tg(&["add", "Task B"]);

    let json = project.run_tg_json(&["doctor"]);
    assert_eq!(json["summary"]["total"], 0);
    assert!(json["issues"].as_array().unwrap().is_empty());
}

#[test]
fn doctor_clean_store_human_output() {
    let project = TestProject::new().unwrap();
    let output = project.run_tg(&["doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No issues found"),
        "Expected healthy message, got: {}",
        stdout
    );
}

#[test]
fn doctor_detects_duplicate_in_both_files() {
    let project = TestProject::new().unwrap();

    // Add an item, then manually copy it to the archive (simulating partial tg done failure)
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap();

    // Read the tasks.jsonl to get the item line
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let tasks_content = fs::read_to_string(&tasks_path).unwrap();
    let item_line = tasks_content.lines().nth(1).unwrap(); // skip header

    // Append the same item to archive
    let archive_path = project.project_dir().join("archive.jsonl");
    let mut archive = fs::read_to_string(&archive_path).unwrap();
    archive.push_str(item_line);
    archive.push('\n');
    fs::write(&archive_path, archive).unwrap();

    let report = project.run_tg_json(&["doctor"]);
    assert!(report["summary"]["total"].as_u64().unwrap() > 0);

    // Should find items_in_both issue
    let issues = report["issues"].as_array().unwrap();
    let both_issues: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "items_in_both")
        .collect();
    assert!(
        !both_issues.is_empty(),
        "Expected items_in_both issue for {}, got: {:?}",
        id,
        issues
    );
}

#[test]
fn doctor_detects_dangling_dep() {
    let project = TestProject::new().unwrap();

    // Add an item, then manually inject a dangling dep
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap();

    // Manually edit the tasks.jsonl to add a fake dep
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = fs::read_to_string(&tasks_path).unwrap();
    let updated = content.replace("\"dependencies\":[]", "\"dependencies\":[\"tg-nonex\"]");
    fs::write(&tasks_path, updated).unwrap();

    let report = project.run_tg_json(&["doctor"]);
    let issues = report["issues"].as_array().unwrap();
    let dangling: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "dangling_dep")
        .collect();
    assert!(
        !dangling.is_empty(),
        "Expected dangling_dep issue for {}, got: {:?}",
        id,
        issues
    );
}

#[test]
fn doctor_detects_cycle() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let b = project.run_tg_json(&["add", "Task B"]);
    let a_id = a["id"].as_str().unwrap().to_string();
    let b_id = b["id"].as_str().unwrap().to_string();

    // Manually inject a cycle: A depends on B, B depends on A
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = fs::read_to_string(&tasks_path).unwrap();
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    for line in &mut lines[1..] {
        if line.contains(&a_id) {
            *line = line.replace(
                "\"dependencies\":[]",
                &format!("\"dependencies\":[\"{}\"]", b_id),
            );
        }
        if line.contains(&b_id) {
            *line = line.replace(
                "\"dependencies\":[]",
                &format!("\"dependencies\":[\"{}\"]", a_id),
            );
        }
    }
    fs::write(&tasks_path, lines.join("\n") + "\n").unwrap();

    let report = project.run_tg_json(&["doctor"]);
    let issues = report["issues"].as_array().unwrap();
    let cycles: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "dependency_cycle")
        .collect();
    assert!(
        !cycles.is_empty(),
        "Expected cycle issue, got: {:?}",
        issues
    );
}

#[test]
fn doctor_fix_removes_duplicates_and_creates_backup() {
    let project = TestProject::new().unwrap();

    // Add an item, then duplicate it in archive
    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap();

    let tasks_path = project.project_dir().join("tasks.jsonl");
    let tasks_content = fs::read_to_string(&tasks_path).unwrap();
    let item_line = tasks_content.lines().nth(1).unwrap();

    let archive_path = project.project_dir().join("archive.jsonl");
    let mut archive = fs::read_to_string(&archive_path).unwrap();
    archive.push_str(item_line);
    archive.push('\n');
    fs::write(&archive_path, archive).unwrap();

    // Run doctor --fix
    let output = project.run_tg(&["--json", "doctor", "--fix"]);
    assert!(output.status.success(), "doctor --fix should succeed");

    // Verify backup files were created
    let project_dir = project.project_dir();
    let backups: Vec<_> = fs::read_dir(&project_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
        .collect();
    assert!(
        backups.len() >= 2,
        "Expected at least 2 backup files, found: {:?}",
        backups.iter().map(|b| b.file_name()).collect::<Vec<_>>()
    );

    // After fix, item should be removed from active (archive is authoritative)
    let post_fix = project.run_tg_json(&["doctor"]);
    // The items_in_both issue should be gone
    let issues = post_fix["issues"].as_array().unwrap();
    let both_issues: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "items_in_both")
        .collect();
    assert!(
        both_issues.is_empty(),
        "Expected items_in_both fixed, got: {:?} for {}",
        issues,
        id
    );
}

#[test]
fn doctor_fix_removes_dangling_deps() {
    let project = TestProject::new().unwrap();

    let json = project.run_tg_json(&["add", "Task A"]);
    let id = json["id"].as_str().unwrap();

    // Inject a dangling dep
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = fs::read_to_string(&tasks_path).unwrap();
    let updated = content.replace("\"dependencies\":[]", "\"dependencies\":[\"tg-nonex\"]");
    fs::write(&tasks_path, updated).unwrap();

    // Run doctor --fix
    let output = project.run_tg(&["--json", "doctor", "--fix"]);
    assert!(output.status.success());

    // Verify the dangling dep is gone
    let show = project.run_tg_json(&["show", id]);
    let deps = show["dependencies"].as_array().unwrap();
    assert!(
        deps.is_empty(),
        "Expected no dependencies after fix, got: {:?}",
        deps
    );
}

#[test]
fn doctor_detects_invalid_status() {
    let project = TestProject::new().unwrap();

    project.run_tg(&["add", "Task A"]);

    // Manually corrupt a status field
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = fs::read_to_string(&tasks_path).unwrap();
    let corrupted = content.replace("\"status\":\"todo\"", "\"status\":\"invalid\"");
    fs::write(&tasks_path, corrupted).unwrap();

    let report = project.run_tg_json(&["doctor"]);
    let issues = report["issues"].as_array().unwrap();

    // Should detect invalid status (via jsonl_syntax since serde will fail to parse)
    // OR detect it via the invalid_status check
    let relevant: Vec<_> = issues
        .iter()
        .filter(|i| i["type"] == "invalid_status" || i["type"] == "jsonl_syntax")
        .collect();
    assert!(
        !relevant.is_empty(),
        "Expected invalid status detection, got: {:?}",
        issues
    );
}
