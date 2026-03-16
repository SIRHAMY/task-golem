mod common;

use std::fs;

use common::TestProject;

#[test]
fn custom_prefix_produces_matching_ids() {
    let project = TestProject::new().unwrap();

    // Create config.yaml with custom prefix
    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "id_prefix: proj\n").unwrap();

    let json = project.run_tg_json(&["add", "Test task"]);
    let id = json["id"].as_str().unwrap();

    assert!(
        id.starts_with("proj-"),
        "Expected ID with prefix 'proj-', got: {}",
        id
    );
    let random_part = &id[5..];
    assert_eq!(
        random_part.len(),
        5,
        "Expected 5 chars after prefix, got: {}",
        random_part
    );
    assert!(
        random_part.chars().all(|c| c.is_ascii_alphanumeric()),
        "Expected alphanumeric chars, got: {}",
        random_part
    );
}

#[test]
fn missing_config_uses_default_prefix() {
    let project = TestProject::new().unwrap();

    // No config.yaml — should use default "tg" prefix
    let json = project.run_tg_json(&["add", "Test task"]);
    let id = json["id"].as_str().unwrap();

    assert!(
        id.starts_with("tg-"),
        "Expected default prefix 'tg-', got: {}",
        id
    );
}

#[test]
fn custom_prefix_items_can_be_resolved() {
    let project = TestProject::new().unwrap();

    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "id_prefix: proj\n").unwrap();

    let added = project.run_tg_json(&["add", "Test task"]);
    let id = added["id"].as_str().unwrap();

    // Should be able to show by full ID
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["id"].as_str().unwrap(), id);

    // Should be able to show by bare hex (prefix resolution tries "tg-" by default,
    // but exact/prefix match should still work for the full ID)
    let output = project.run_tg(&["--json", "show", id]);
    assert!(output.status.success());
}

#[test]
fn empty_config_uses_default_prefix() {
    let project = TestProject::new().unwrap();

    // Write an empty config
    let config_path = project.project_dir().join("config.yaml");
    fs::write(&config_path, "").unwrap();

    let json = project.run_tg_json(&["add", "Test task"]);
    let id = json["id"].as_str().unwrap();

    assert!(
        id.starts_with("tg-"),
        "Empty config should use default prefix, got: {}",
        id
    );
}
