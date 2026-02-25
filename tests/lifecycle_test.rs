mod common;

use common::TestProject;

/// Full lifecycle verification test:
/// tg init && tg add "Task A" && tg add "Task B" --dep <A-id> && tg list --json
/// && tg show <B-id> --json && tg edit <A-id> --priority 5
/// && tg rm <A-id> --force --clear-deps && tg show <B-id> --json
/// (verify B's deps no longer contain A)
#[test]
fn full_crud_lifecycle() {
    let proj = TestProject::new().unwrap();

    // Add Task A
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    assert_eq!(json_a["status"], "todo");

    // Add Task B with dep on A
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();
    assert_eq!(json_b["dependencies"], serde_json::json!([&id_a]));

    // List should show both
    let list = proj.run_tg_json(&["list"]);
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Show B
    let show_b = proj.run_tg_json(&["show", &id_b]);
    assert_eq!(show_b["title"], "Task B");
    assert_eq!(show_b["dependencies"], serde_json::json!([&id_a]));

    // Edit A priority
    let edit_a = proj.run_tg_json(&["edit", &id_a, "--priority", "5"]);
    assert_eq!(edit_a["priority"], 5);

    // Remove A with --force --clear-deps
    let rm = proj.run_tg_json(&["rm", &id_a, "--force", "--clear-deps"]);
    assert_eq!(rm["removed"], id_a);

    // Show B — deps should no longer contain A
    let show_b_after = proj.run_tg_json(&["show", &id_b]);
    assert_eq!(show_b_after["dependencies"], serde_json::json!([]));

    // List should show only B
    let list_after = proj.run_tg_json(&["list"]);
    let arr_after = list_after.as_array().unwrap();
    assert_eq!(arr_after.len(), 1);
    assert_eq!(arr_after[0]["id"], id_b);
}

/// Extension fields lifecycle: add with extension, show to verify, edit to modify
#[test]
fn extension_fields_lifecycle() {
    let proj = TestProject::new().unwrap();

    let json = proj.run_tg_json(&["add", "Test", "--set", "x-meta.key=42"]);
    let id = json["id"].as_str().unwrap();
    assert_eq!(json["x-meta"]["key"], 42);

    let show = proj.run_tg_json(&["show", id]);
    assert_eq!(show["x-meta"]["key"], 42);

    // Edit: add another nested key
    let edit = proj.run_tg_json(&["edit", id, "--set", "x-meta.name=test"]);
    assert_eq!(edit["x-meta"]["key"], 42);
    assert_eq!(edit["x-meta"]["name"], "test");
}

/// Cycle detection: A -> B -> add dep A -> B should fail
#[test]
fn cycle_detection_integration() {
    let proj = TestProject::new().unwrap();

    let json_a = proj.run_tg_json(&["add", "A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();

    let json_b = proj.run_tg_json(&["add", "B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    // Try to create cycle: edit A to depend on B
    let output = proj.run_tg(&["edit", &id_a, "--add-dep", &id_b]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cycle") || stderr.contains("Cycle"),
        "Should mention cycle: {}",
        stderr
    );
}

/// Per-command JSON schema validation for all commands
#[test]
fn json_schema_validation_all_commands() {
    let proj = TestProject::new().unwrap();

    // add
    let add = proj.run_tg_json(&["add", "Task"]);
    validate_item_schema(&add);

    let id = add["id"].as_str().unwrap();

    // show
    let show = proj.run_tg_json(&["show", id]);
    validate_item_schema(&show);

    // edit
    let edit = proj.run_tg_json(&["edit", id, "--title", "Updated"]);
    validate_item_schema(&edit);

    // list
    let list = proj.run_tg_json(&["list"]);
    let arr = list.as_array().unwrap();
    for item in arr {
        validate_item_schema(item);
    }

    // rm
    let rm = proj.run_tg_json(&["rm", id]);
    assert!(rm["removed"].is_string());
}

fn validate_item_schema(json: &serde_json::Value) {
    // ID format
    let id = json["id"].as_str().unwrap();
    assert!(id.starts_with("tg-"), "ID format: {}", id);
    assert_eq!(id.len(), 8, "ID length: {}", id);

    // Status enum
    let status = json["status"].as_str().unwrap();
    assert!(
        ["todo", "doing", "done", "blocked"].contains(&status),
        "Status: {}",
        status
    );

    // Timestamps
    assert!(
        chrono::DateTime::parse_from_rfc3339(json["created_at"].as_str().unwrap()).is_ok()
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(json["updated_at"].as_str().unwrap()).is_ok()
    );

    // Null fields present
    assert!(json.get("description").is_some());
    assert!(json.get("blocked_reason").is_some());
    assert!(json.get("blocked_from_status").is_some());
    assert!(json.get("claimed_by").is_some());
    assert!(json.get("claimed_at").is_some());

    // Array fields
    assert!(json["tags"].is_array());
    assert!(json["dependencies"].is_array());
}
