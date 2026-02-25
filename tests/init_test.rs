mod common;

use std::fs;

use predicates::prelude::*;

#[test]
fn init_creates_directory_and_files() {
    let project = common::TestProject::new().unwrap();
    let dir = project.project_dir();

    assert!(dir.exists(), ".task-golem/ should exist");
    assert!(dir.join("tasks.jsonl").exists(), "tasks.jsonl should exist");
    assert!(
        dir.join("archive.jsonl").exists(),
        "archive.jsonl should exist"
    );
    assert!(dir.join("tasks.lock").exists(), "tasks.lock should exist");
}

#[test]
fn init_schema_headers_correct() {
    let project = common::TestProject::new().unwrap();
    let dir = project.project_dir();

    let tasks_content = fs::read_to_string(dir.join("tasks.jsonl")).unwrap();
    let archive_content = fs::read_to_string(dir.join("archive.jsonl")).unwrap();

    let tasks_header: serde_json::Value =
        serde_json::from_str(tasks_content.lines().next().unwrap()).unwrap();
    assert_eq!(tasks_header["schema_version"], 1);

    let archive_header: serde_json::Value =
        serde_json::from_str(archive_content.lines().next().unwrap()).unwrap();
    assert_eq!(archive_header["schema_version"], 1);
}

#[test]
fn init_error_on_existing_without_force() {
    let project = common::TestProject::new().unwrap();

    // Try to init again without --force
    let output = project.run_tg(&["init"]);
    assert!(
        !output.status.success(),
        "Should fail without --force on existing project"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "Should exit with code 1 on user error"
    );
}

#[test]
fn init_force_reinitializes_with_warning() {
    let project = common::TestProject::new().unwrap();

    // Write some data to prove it gets overwritten
    let tasks_path = project.project_dir().join("tasks.jsonl");
    fs::write(&tasks_path, "garbage data").unwrap();

    // Reinitialize with --force
    let output = project.run_tg(&["init", "--force"]);
    assert!(output.status.success(), "Should succeed with --force");

    // Check warning on stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    let pred = predicates::str::contains("Reinitializing")
        .or(predicates::str::contains("overwrite"))
        .or(predicates::str::contains("Warning"));
    assert!(
        pred.eval(&stderr),
        "Should warn about reinitializing on stderr: {}",
        stderr
    );

    // Verify files are valid again
    let tasks_content = fs::read_to_string(&tasks_path).unwrap();
    let header: serde_json::Value =
        serde_json::from_str(tasks_content.lines().next().unwrap()).unwrap();
    assert_eq!(header["schema_version"], 1);
}

#[test]
fn init_json_output() {
    let project = common::TestProject::new_uninit().unwrap();

    let output = project.run_tg(&["--json", "init"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["initialized"], true);
    assert_eq!(json["path"], ".task-golem/");
}

#[test]
fn init_json_output_matches_schema() {
    let json = {
        let project = common::TestProject::new_uninit().unwrap();
        project.run_tg_json(&["init"])
    };

    assert_eq!(json["initialized"], true);
    assert_eq!(json["path"], ".task-golem/");
}
