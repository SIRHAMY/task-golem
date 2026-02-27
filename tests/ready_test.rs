mod common;

use common::TestProject;

#[test]
fn ready_items_with_no_deps() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    project.run_tg_json(&["add", "Task A"]);
    project.run_tg_json(&["add", "Task B"]);

    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 2);
    Ok(())
}

#[test]
fn ready_items_with_met_deps() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    let b = project.run_tg_json(&["add", "Task B", "--dep", a_id]);
    let b_id = b["id"].as_str().unwrap();

    // B depends on A; A is not done → only A is ready
    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), a_id);

    // Complete A → B should become ready
    project.run_tg_json(&["done", a_id]);
    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), b_id);

    Ok(())
}

#[test]
fn ready_items_with_unmet_deps() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    project.run_tg_json(&["add", "Task B", "--dep", a_id]);

    // A is doing (not done) → B is not ready
    project.run_tg_json(&["do", a_id]);
    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 0);

    Ok(())
}

#[test]
fn ready_dep_on_archived_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    let b = project.run_tg_json(&["add", "Task B", "--dep", a_id]);
    let b_id = b["id"].as_str().unwrap();

    // Archive A (done moves to archive)
    project.run_tg_json(&["done", a_id]);

    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), b_id);

    Ok(())
}

#[test]
fn ready_dep_on_nonexistent_id_not_ready() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Create an item first, then manually inject a dangling dep via JSONL
    let _added = project.run_tg_json(&["add", "Task A"]);

    // Manually edit JSONL to add a dep on non-existent ID
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = std::fs::read_to_string(&tasks_path)?;
    let modified = content.replace(r#""dependencies":[]"#, r#""dependencies":["tg-zzzzz"]"#);
    std::fs::write(&tasks_path, modified)?;

    // Should not be ready (unmet dep), and warning emitted
    let output = project.run_tg(&["--json", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ready: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(ready.as_array().unwrap().is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tg-zzzzz"),
        "Expected warning about tg-zzzzz, got: {}",
        stderr
    );

    Ok(())
}

#[test]
fn ready_sort_order() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add items with different priorities
    let low = project.run_tg_json(&["add", "Low prio", "--priority", "1"]);
    let high = project.run_tg_json(&["add", "High prio", "--priority", "10"]);
    let med = project.run_tg_json(&["add", "Med prio", "--priority", "5"]);

    let ready = project.run_tg_json(&["ready"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 3);
    // Sorted by priority desc
    assert_eq!(items[0]["id"], high["id"]);
    assert_eq!(items[1]["id"], med["id"]);
    assert_eq!(items[2]["id"], low["id"]);

    Ok(())
}

#[test]
fn ready_empty_queue() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let ready = project.run_tg_json(&["ready"]);
    assert!(ready.as_array().unwrap().is_empty());

    Ok(())
}

#[test]
fn ready_completing_dep_unlocks_downstream() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    let b = project.run_tg_json(&["add", "Task B", "--dep", a_id]);
    let b_id = b["id"].as_str().unwrap();
    let c = project.run_tg_json(&["add", "Task C", "--dep", b_id]);
    let c_id = c["id"].as_str().unwrap();

    // Only A ready initially
    let ready = project.run_tg_json(&["ready"]);
    assert_eq!(ready.as_array().unwrap().len(), 1);
    assert_eq!(ready[0]["id"].as_str().unwrap(), a_id);

    // Complete A → B becomes ready
    project.run_tg_json(&["done", a_id]);
    let ready = project.run_tg_json(&["ready"]);
    assert_eq!(ready.as_array().unwrap().len(), 1);
    assert_eq!(ready[0]["id"].as_str().unwrap(), b_id);

    // Complete B → C becomes ready
    project.run_tg_json(&["done", b_id]);
    let ready = project.run_tg_json(&["ready"]);
    assert_eq!(ready.as_array().unwrap().len(), 1);
    assert_eq!(ready[0]["id"].as_str().unwrap(), c_id);

    Ok(())
}

#[test]
fn ready_limit() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    project.run_tg_json(&["add", "Task A", "--priority", "3"]);
    project.run_tg_json(&["add", "Task B", "--priority", "2"]);
    project.run_tg_json(&["add", "Task C", "--priority", "1"]);

    let ready = project.run_tg_json(&["ready", "--limit", "2"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Highest priority first
    assert_eq!(items[0]["priority"], 3);
    assert_eq!(items[1]["priority"], 2);

    Ok(())
}

#[test]
fn ready_include_stale() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    // Move to doing
    project.run_tg_json(&["do", a_id]);

    // Manually set old updated_at in JSONL to simulate stale item
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = std::fs::read_to_string(&tasks_path)?;
    let old_time = "2020-01-01T00:00:00Z";
    let modified = content.replace(
        &format!(r#""updated_at":"{}""#, {
            // Find the current updated_at value
            let lines: Vec<&str> = content.lines().collect();
            let item_line = lines.iter().find(|l| l.contains(a_id)).unwrap();
            let item: serde_json::Value = serde_json::from_str(item_line).unwrap();
            item["updated_at"].as_str().unwrap().to_string()
        }),
        &format!(r#""updated_at":"{}""#, old_time),
    );
    std::fs::write(&tasks_path, modified)?;

    // Without --include-stale: doing items not shown
    let ready = project.run_tg_json(&["ready"]);
    assert!(ready.as_array().unwrap().is_empty());

    // With --include-stale=1s: stale doing item should appear
    let ready = project.run_tg_json(&["ready", "--include-stale", "1s"]);
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), a_id);
    assert_eq!(items[0]["status"], "doing");

    Ok(())
}

#[test]
fn ready_include_stale_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    // Move to doing — updated_at is now
    project.run_tg_json(&["do", a_id]);

    // Items updated just now should NOT be stale with any reasonable duration
    let ready = project.run_tg_json(&["ready", "--include-stale", "1h"]);
    let items = ready.as_array().unwrap();
    // The item was just updated, so it's NOT stale
    assert!(items.is_empty());

    Ok(())
}

#[test]
fn ready_doing_items_excluded() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    project.run_tg_json(&["do", a_id]);

    // Doing items should not be in the ready queue
    let ready = project.run_tg_json(&["ready"]);
    assert!(ready.as_array().unwrap().is_empty());

    Ok(())
}

#[test]
fn ready_blocked_items_excluded() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    project.run_tg_json(&["block", a_id]);

    // Blocked items should not be in the ready queue
    let ready = project.run_tg_json(&["ready"]);
    assert!(ready.as_array().unwrap().is_empty());

    Ok(())
}

#[test]
fn ready_json_schema() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    project.run_tg_json(&["add", "Test item"]);

    let ready = project.run_tg_json(&["ready"]);
    assert!(ready.is_array());
    let items = ready.as_array().unwrap();
    assert_eq!(items.len(), 1);

    // Validate item schema
    let item = &items[0];
    assert!(item["id"].is_string());
    assert!(item["status"].is_string());
    assert_eq!(item["status"], "todo");

    Ok(())
}
