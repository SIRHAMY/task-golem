mod common;

use common::TestProject;

#[test]
fn edit_title() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Original title"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--title", "New title"]);
    assert_eq!(edit_json["title"], "New title");
}

#[test]
fn edit_priority() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--priority", "10"]);
    assert_eq!(edit_json["priority"], 10);
}

#[test]
fn edit_description() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--description", "New desc"]);
    assert_eq!(edit_json["description"], "New desc");
}

#[test]
fn edit_add_dep() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B"]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    let edit_json = proj.run_tg_json(&["edit", &id_b, "--add-dep", &id_a]);
    assert_eq!(edit_json["dependencies"], serde_json::json!([id_a]));
}

#[test]
fn edit_add_dep_cycle_rejection() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    // Try to add A -> B (creates cycle: A -> B -> A)
    let output = proj.run_tg(&["edit", &id_a, "--add-dep", &id_b]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn edit_rm_dep() {
    let proj = TestProject::new().unwrap();
    let json_a = proj.run_tg_json(&["add", "Task A"]);
    let id_a = json_a["id"].as_str().unwrap().to_string();
    let json_b = proj.run_tg_json(&["add", "Task B", "--dep", &id_a]);
    let id_b = json_b["id"].as_str().unwrap().to_string();

    let edit_json = proj.run_tg_json(&["edit", &id_b, "--rm-dep", &id_a]);
    assert_eq!(edit_json["dependencies"], serde_json::json!([]));
}

#[test]
fn edit_add_rm_tags() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task", "--tag", "old"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--add-tag", "new", "--rm-tag", "old"]);
    assert_eq!(edit_json["tags"], serde_json::json!(["new"]));
}

#[test]
fn edit_set_extension() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--set", "x-meta.key=42"]);
    assert_eq!(edit_json["x-meta"]["key"], 42);
}

#[test]
fn edit_delete_extension() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task", "--set", "x-foo=bar"]);
    let id = add_json["id"].as_str().unwrap();

    let edit_json = proj.run_tg_json(&["edit", id, "--set", "x-foo="]);
    assert!(edit_json.get("x-foo").is_none());
}

#[test]
fn edit_overwrite_conflict() {
    let proj = TestProject::new().unwrap();
    // Create item with x-foo as string
    let add_json = proj.run_tg_json(&["add", "Task", "--set", "x-foo=hello"]);
    let id = add_json["id"].as_str().unwrap();

    // Overwrite x-foo with nested object
    let edit_json = proj.run_tg_json(&["edit", id, "--set", "x-foo.bar=1"]);
    assert_eq!(edit_json["x-foo"], serde_json::json!({"bar": 1}));
}

#[test]
fn edit_updated_at_changes() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task"]);
    let id = add_json["id"].as_str().unwrap();
    let original_updated = add_json["updated_at"].as_str().unwrap().to_string();

    // Wait a tiny bit to ensure time difference
    std::thread::sleep(std::time::Duration::from_millis(10));

    let edit_json = proj.run_tg_json(&["edit", id, "--title", "Updated"]);
    let new_updated = edit_json["updated_at"].as_str().unwrap();
    assert_ne!(original_updated, new_updated);
}

#[test]
fn edit_nonexistent_exits_1() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["edit", "tg-zzz99", "--title", "New"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}
