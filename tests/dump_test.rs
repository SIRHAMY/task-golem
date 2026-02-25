mod common;

use common::TestProject;

#[test]
fn dump_json_produces_valid_json() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add some items
    project.run_tg_json(&["add", "Active task"]);
    let added = project.run_tg_json(&["add", "Done task"]);
    let id = added["id"].as_str().unwrap().to_string();
    project.run_tg_json(&["done", &id]);

    // Dump as JSON (default)
    let output = project.run_tg(&["dump"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;

    // Verify structure
    assert!(json["active"].is_array());
    assert!(json["archive"].is_array());
    assert_eq!(json["active"].as_array().unwrap().len(), 1);
    assert_eq!(json["archive"].as_array().unwrap().len(), 1);

    // Verify items have expected fields
    let active_item = &json["active"][0];
    assert!(active_item["id"].is_string());
    assert!(active_item["title"].is_string());
    assert!(active_item["status"].is_string());

    let archive_item = &json["archive"][0];
    assert_eq!(archive_item["id"].as_str().unwrap(), &id);
    assert_eq!(archive_item["status"], "done");

    Ok(())
}

#[test]
fn dump_yaml_produces_valid_yaml() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    project.run_tg_json(&["add", "Test task"]);

    let output = project.run_tg(&["dump", "--yaml"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // YAML output should contain key markers
    assert!(stdout.contains("active:"));
    assert!(stdout.contains("archive:"));

    // Verify it parses as valid YAML
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout)?;
    assert!(parsed["active"].is_sequence());
    assert!(parsed["archive"].is_sequence());

    Ok(())
}

#[test]
fn dump_empty_project() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["dump"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(json["active"].as_array().unwrap().is_empty());
    assert!(json["archive"].as_array().unwrap().is_empty());

    Ok(())
}

#[test]
fn dump_json_includes_all_items_from_both_stores() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add active items
    project.run_tg_json(&["add", "Active 1"]);
    project.run_tg_json(&["add", "Active 2"]);
    project.run_tg_json(&["add", "Active 3"]);

    // Add and archive items
    let d1 = project.run_tg_json(&["add", "Done 1"]);
    project.run_tg_json(&["done", d1["id"].as_str().unwrap()]);
    let d2 = project.run_tg_json(&["add", "Done 2"]);
    project.run_tg_json(&["done", d2["id"].as_str().unwrap()]);

    let output = project.run_tg(&["dump"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(json["active"].as_array().unwrap().len(), 3);
    assert_eq!(json["archive"].as_array().unwrap().len(), 2);

    Ok(())
}
