mod common;

use common::TestProject;

#[test]
fn add_basic() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["add", "My first task"]);
    assert!(output.status.success(), "tg add should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created item:"));
    assert!(stdout.contains("My first task"));
}

#[test]
fn add_json_output_schema() {
    let proj = TestProject::new().unwrap();
    let json = proj.run_tg_json(&["add", "Test task"]);
    // Verify required fields
    let id = json["id"].as_str().unwrap();
    assert!(id.starts_with("tg-"), "ID should start with tg-: {}", id);
    assert_eq!(id.len(), 8, "ID should be 8 chars: {}", id);
    assert!(
        id[3..].chars().all(|c| c.is_ascii_hexdigit()),
        "ID hex part should be valid: {}",
        id
    );
    assert_eq!(json["status"], "todo");
    assert_eq!(json["title"], "Test task");
    assert_eq!(json["priority"], 0);
    assert!(json["description"].is_null());
    assert!(json["tags"].is_array());
    assert!(json["dependencies"].is_array());
    assert!(json["created_at"].is_string());
    assert!(json["updated_at"].is_string());
    assert!(json["blocked_reason"].is_null());
    assert!(json["blocked_from_status"].is_null());
    assert!(json["claimed_by"].is_null());
    assert!(json["claimed_at"].is_null());
}

#[test]
fn add_with_all_optional_fields() {
    let proj = TestProject::new().unwrap();
    let json = proj.run_tg_json(&[
        "add",
        "Full task",
        "--description",
        "A detailed description",
        "--priority",
        "5",
        "--tag",
        "backend",
        "--tag",
        "urgent",
        "--set",
        "x-meta.key=42",
    ]);
    assert_eq!(json["title"], "Full task");
    assert_eq!(json["description"], "A detailed description");
    assert_eq!(json["priority"], 5);
    assert_eq!(json["tags"], serde_json::json!(["backend", "urgent"]));
    assert_eq!(json["x-meta"]["key"], 42);
}

#[test]
fn add_title_newline_rejection() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["add", "Bad\ntitle"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn add_dep_on_nonexistent_warns() {
    let proj = TestProject::new().unwrap();
    // First add a task to get a valid ID to use as reference
    let json = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json["id"].as_str().unwrap().to_string();

    // Add task with dep on a non-existent ID - should fail since ID doesn't resolve
    let output = proj.run_tg(&["add", "Task B", "--dep", "tg-zzz99"]);
    // This should fail because tg-zzz99 doesn't resolve
    assert!(!output.status.success());

    // But dep on existing item should work
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    assert_eq!(json_b["dependencies"], serde_json::json!([id_a]));
}

#[test]
fn add_multiple_create_distinct_ids() {
    let proj = TestProject::new().unwrap();
    let json1 = proj.run_tg_json(&["add", "Task 1"]);
    let json2 = proj.run_tg_json(&["add", "Task 2"]);
    let json3 = proj.run_tg_json(&["add", "Task 3"]);

    let id1 = json1["id"].as_str().unwrap();
    let id2 = json2["id"].as_str().unwrap();
    let id3 = json3["id"].as_str().unwrap();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

#[test]
fn add_with_dep_on_existing() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();

    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    assert_eq!(json_b["dependencies"], serde_json::json!([id_a]));
}

#[test]
fn add_timestamps_are_iso8601() {
    let proj = TestProject::new().unwrap();
    let json = proj.run_tg_json(&["add", "Timestamped task"]);
    let created = json["created_at"].as_str().unwrap();
    let updated = json["updated_at"].as_str().unwrap();
    // Should parse as ISO 8601
    assert!(
        chrono::DateTime::parse_from_rfc3339(created).is_ok(),
        "created_at should be ISO 8601: {}",
        created
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(updated).is_ok(),
        "updated_at should be ISO 8601: {}",
        updated
    );
}
