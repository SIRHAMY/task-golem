mod common;

use std::fs;

use common::TestProject;

/// Helper: generate a bulk JSONL file with N items via domain-layer insertion.
fn generate_items_jsonl(n: usize, prefix: &str, status: &str) -> String {
    let mut lines = Vec::new();
    lines.push("{\"schema_version\":1}".to_string());

    let now = "2026-02-24T12:00:00Z";
    for i in 0..n {
        let id = format!("{}-{:05x}", prefix, i);
        let item = serde_json::json!({
            "id": id,
            "title": format!("Item {}", i),
            "status": status,
            "priority": (i % 10) as i64,
            "description": null,
            "tags": [],
            "dependencies": [],
            "created_at": now,
            "updated_at": now,
            "blocked_reason": null,
            "blocked_from_status": null,
            "claimed_by": null,
            "claimed_at": null,
        });
        lines.push(serde_json::to_string(&item).unwrap());
    }

    lines.join("\n") + "\n"
}

#[test]
fn large_store_2000_active_items() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Write 2000 active items directly to JSONL (bypass CLI for speed)
    let tasks_path = project.path().join(".task-golem/tasks.jsonl");
    let content = generate_items_jsonl(2000, "tg", "todo");
    fs::write(&tasks_path, content)?;

    // Verify list works
    let output = project.run_tg(&["--json", "list"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json.as_array().unwrap().len(), 2000);

    // Verify ready works
    let output = project.run_tg(&["--json", "ready"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json.as_array().unwrap().len(), 2000);

    // Verify next works
    let output = project.run_tg(&["--json", "next"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert!(json["id"].is_string());

    // Verify show works on a specific item
    let output = project.run_tg(&["--json", "show", "tg-00000"]);
    assert!(output.status.success());

    // Verify doctor works
    let doctor = project.run_tg_json(&["doctor"]);
    assert_eq!(doctor["summary"]["total"], 0);

    Ok(())
}

#[test]
fn large_store_5000_archive_items() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Write 5000 archived items
    let archive_path = project.path().join(".task-golem/archive.jsonl");
    let content = generate_items_jsonl(5000, "tg", "done");
    fs::write(&archive_path, content)?;

    // Verify list --status done loads all archive items
    let output = project.run_tg(&["--json", "list", "--status", "done"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json.as_array().unwrap().len(), 5000);

    // Verify show works on an archived item
    let output = project.run_tg(&["--json", "show", "tg-00000"]);
    assert!(output.status.success());

    // Verify dump includes all items
    let output = project.run_tg(&["dump"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["archive"].as_array().unwrap().len(), 5000);

    Ok(())
}

#[test]
fn large_store_mixed_2000_active_5000_archive() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Write active items
    let tasks_path = project.path().join(".task-golem/tasks.jsonl");
    let active_content = generate_items_jsonl(2000, "tg", "todo");
    fs::write(&tasks_path, active_content)?;

    // Write archive items (using different IDs to avoid collisions)
    let archive_path = project.path().join(".task-golem/archive.jsonl");
    let archive_content = generate_items_jsonl(5000, "ar", "done");
    fs::write(&archive_path, archive_content)?;

    // Adding a new item should work even with large existing stores
    let output = project.run_tg(&["--json", "add", "New item"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert!(json["id"].is_string());

    // Ready queue should include all 2000 todo items (no deps)
    let output = project.run_tg(&["--json", "ready", "--limit", "10"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json.as_array().unwrap().len(), 10);

    // Dump should include all items
    let output = project.run_tg(&["dump"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    // 2000 + 1 new item
    assert_eq!(json["active"].as_array().unwrap().len(), 2001);
    assert_eq!(json["archive"].as_array().unwrap().len(), 5000);

    Ok(())
}
