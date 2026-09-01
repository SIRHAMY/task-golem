mod common;

use common::TestProject;

#[test]
fn rm_basic() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task to remove"]);
    let id = add_json["id"].as_str().unwrap();

    let rm_json = proj.run_tg_json(&["rm", id]);
    assert_eq!(rm_json["removed"], id);

    // Verify it's gone
    let output = proj.run_tg(&["show", id]);
    assert!(!output.status.success());
}

#[test]
fn rm_with_dependents_error() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let _id_b = json_b["id"].as_str().unwrap().to_string();

    // Try to remove A (B depends on it)
    let output = proj.run_tg(&["rm", &id_a]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Remove those dependency edges first"));
}

#[test]
fn rm_force_cannot_leave_dangling_deps() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    // Act
    let output = proj.run_tg(&["--json", "rm", &id_a, "--force"]);

    // Assert
    assert!(!output.status.success());
    assert_eq!(proj.run_tg_json(&["show", &id_a])["id"], id_a);
    let show_b = proj.run_tg_json(&["show", &id_b]);
    assert_eq!(show_b["dependencies"], serde_json::json!([id_a]));
}

#[test]
fn rm_succeeds_after_dependency_edge_is_removed() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    // Act
    proj.run_tg_json(&["dep", "rm", &id_b, &id_a]);
    let rm_json = proj.run_tg_json(&["rm", &id_a]);

    // Assert
    assert_eq!(rm_json["removed"], id_a);
    let show_b = proj.run_tg_json(&["show", &id_b]);
    assert_eq!(show_b["dependencies"], serde_json::json!([]));
}

#[test]
fn rm_clear_deps_without_force_no_dependents() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();

    // --clear-deps without --force on item with no dependents works fine
    let rm_json = proj.run_tg_json(&["rm", id, "--clear-deps"]);
    assert_eq!(rm_json["removed"], id);
}

#[test]
fn rm_nonexistent_exits_1() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["rm", "tg-zzz99"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn rm_rejected_when_children_exist() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Parent"]);
    let pid = p["id"].as_str().unwrap().to_string();
    proj.run_tg_json(&["add", "Child", "--parent", &pid]);

    // Plain rm is rejected — --force does NOT bypass the children check.
    let output = proj.run_tg(&["rm", &pid]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);

    let output_forced = proj.run_tg(&["rm", &pid, "--force"]);
    assert!(!output_forced.status.success());
    let stderr = String::from_utf8_lossy(&output_forced.stderr);
    assert!(stderr.to_lowercase().contains("children"));
}

#[test]
fn rm_json_output_schema() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();

    let rm_json = proj.run_tg_json(&["rm", id]);
    assert!(rm_json["removed"].is_string());
    // cleared_deps_from should not be present when empty (skip_serializing_if)
    assert!(rm_json.get("cleared_deps_from").is_none());
}
