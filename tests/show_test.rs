mod common;

use common::TestProject;

#[test]
fn show_full_id() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();

    let show_json = proj.run_tg_json(&["show", id]);
    assert_eq!(show_json["id"], id);
    assert_eq!(show_json["title"], "Show me");
}

#[test]
fn show_bare_hex() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();
    let bare_hex = &id[3..]; // strip "tg-"

    let show_json = proj.run_tg_json(&["show", bare_hex]);
    assert_eq!(show_json["id"], id);
}

#[test]
fn show_prefix_match() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();
    let prefix = &id[3..6]; // first 3 hex chars

    let show_json = proj.run_tg_json(&["show", prefix]);
    assert_eq!(show_json["id"], id);
}

#[test]
fn show_not_found() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["show", "tg-zzz99"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn show_json_schema_complete() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Complete item", "--set", "x-test=true"]);
    let id = add_json["id"].as_str().unwrap();

    let show_json = proj.run_tg_json(&["show", id]);
    // All fields present including null fields
    assert!(show_json["id"].is_string());
    assert!(show_json["title"].is_string());
    assert!(show_json["status"].is_string());
    assert!(show_json["priority"].is_number());
    assert!(show_json.get("description").is_some()); // null but present
    assert!(show_json["tags"].is_array());
    assert!(show_json["dependencies"].is_array());
    assert!(show_json["created_at"].is_string());
    assert!(show_json["updated_at"].is_string());
    assert!(show_json.get("blocked_reason").is_some());
    assert!(show_json.get("blocked_from_status").is_some());
    assert!(show_json.get("claimed_by").is_some());
    assert!(show_json.get("claimed_at").is_some());
    assert_eq!(show_json["x-test"], true);
}

#[test]
fn show_archive_fallback() {
    let proj = TestProject::new().unwrap();
    // Manually write an item to the archive for testing
    let archive_path = proj.project_dir().join("archive.jsonl");
    let archive_content = r#"{"schema_version":1}
{"id":"tg-arc01","title":"Archived item","status":"done","priority":0,"description":null,"tags":[],"dependencies":[],"created_at":"2026-02-24T12:00:00Z","updated_at":"2026-02-24T12:00:00Z","blocked_reason":null,"blocked_from_status":null,"claimed_by":null,"claimed_at":null}
"#;
    std::fs::write(&archive_path, archive_content).unwrap();

    let show_json = proj.run_tg_json(&["show", "tg-arc01"]);
    assert_eq!(show_json["id"], "tg-arc01");
    assert_eq!(show_json["title"], "Archived item");
    assert_eq!(show_json["status"], "done");
}
