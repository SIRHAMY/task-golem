mod common;

use std::fs;

use common::TestProject;

#[test]
fn legacy_id_prefix_config_is_rejected() {
    let project = TestProject::new().unwrap();
    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "id_prefix: proj\n").unwrap();

    let output = project.run_tg(&["--json", "add", "Test task"]);
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "invalid_input");
    assert!(error["error"].as_str().unwrap().contains("id_prefix"));
}

#[test]
fn legacy_id_len_config_is_rejected() {
    let project = TestProject::new().unwrap();
    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "id_len: 8\n").unwrap();

    let output = project.run_tg(&["--json", "add", "Test task"]);
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "invalid_input");
    assert!(error["error"].as_str().unwrap().contains("id_len"));
}

#[test]
fn missing_config_generates_canonical_uuid_v7() {
    let project = TestProject::new().unwrap();

    let added = project.run_tg_json(&["add", "Test task"]);
    let id = added["id"].as_str().unwrap();
    task_golem::validate_id(id).unwrap();
}

#[test]
fn empty_config_generates_canonical_uuid_v7() {
    let project = TestProject::new().unwrap();

    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "").unwrap();

    let json = project.run_tg_json(&["add", "Test task"]);
    let id = json["id"].as_str().unwrap();
    task_golem::validate_id(id).unwrap();
}
