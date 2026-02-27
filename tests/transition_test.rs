mod common;

use common::TestProject;

// === Valid transitions ===

#[test]
fn transition_todo_to_doing() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["do", id]);
    assert_eq!(result["status"], "doing");
    assert_eq!(result["id"], id);
    Ok(())
}

#[test]
fn transition_todo_to_done() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["done", id]);
    assert_eq!(result["status"], "done");
    assert_eq!(result["id"], id);
    // Verify claims are null (never set)
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn transition_todo_to_blocked() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["block", id, "--reason", "waiting on API"]);
    assert_eq!(result["status"], "blocked");
    assert_eq!(result["blocked_reason"], "waiting on API");
    assert_eq!(result["blocked_from_status"], "todo");
    Ok(())
}

#[test]
fn transition_doing_to_done() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id]);
    let result = project.run_tg_json(&["done", id]);
    assert_eq!(result["status"], "done");
    // Claims should be cleared
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn transition_doing_to_blocked() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let result = project.run_tg_json(&["block", id, "--reason", "blocker found"]);
    assert_eq!(result["status"], "blocked");
    assert_eq!(result["blocked_from_status"], "doing");
    assert_eq!(result["blocked_reason"], "blocker found");
    // Claims should be cleared when blocking from doing
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn transition_doing_to_todo() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let result = project.run_tg_json(&["todo", id]);
    assert_eq!(result["status"], "todo");
    // Claims should be cleared
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

// === Invalid transitions ===

#[test]
fn transition_done_to_anything_fails() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // Move to done (archives)
    project.run_tg_json(&["done", id]);

    // All further transitions should fail (item is archived, not in active store)
    let output = project.run_tg(&["--json", "do", id]);
    assert!(!output.status.success());

    let output = project.run_tg(&["--json", "todo", id]);
    assert!(!output.status.success());

    let output = project.run_tg(&["--json", "block", id]);
    assert!(!output.status.success());

    Ok(())
}

#[test]
fn transition_blocked_to_doing_fails() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["block", id]);

    let output = project.run_tg(&["--json", "do", id]);
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn transition_blocked_to_done_fails() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["block", id]);

    let output = project.run_tg(&["--json", "done", id]);
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn transition_blocked_to_blocked_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["block", id]);

    let output = project.run_tg(&["--json", "block", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["idempotent"], true);
    assert_eq!(json["previous_state"], "blocked");
    Ok(())
}

// === Unblock tests ===

#[test]
fn unblock_restores_todo() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["block", id]);
    let result = project.run_tg_json(&["unblock", id]);
    assert_eq!(result["status"], "todo");
    assert!(result["blocked_reason"].is_null());
    assert!(result["blocked_from_status"].is_null());
    Ok(())
}

#[test]
fn unblock_restores_doing() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id]);
    project.run_tg_json(&["block", id]);
    let result = project.run_tg_json(&["unblock", id]);
    assert_eq!(result["status"], "doing");
    assert!(result["blocked_reason"].is_null());
    assert!(result["blocked_from_status"].is_null());
    Ok(())
}

#[test]
fn unblock_on_non_blocked_fails() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let output = project.run_tg(&["--json", "unblock", id]);
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn block_unblock_cycle() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // do -> block -> unblock -> should be doing again
    project.run_tg_json(&["do", id]);
    project.run_tg_json(&["block", id, "--reason", "waiting"]);
    let result = project.run_tg_json(&["unblock", id]);
    assert_eq!(result["status"], "doing");
    Ok(())
}

// === Claim semantics ===

#[test]
fn claim_set_on_do() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    assert_eq!(result["claimed_by"], "agent-1");
    assert!(!result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn do_without_claim_no_claim_fields() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["do", id]);
    assert_eq!(result["status"], "doing");
    // Without --claim, claim fields should remain null
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn claim_conflict_different_agent() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);

    // Agent-2 tries to claim — should fail
    let output = project.run_tg(&["--json", "do", id, "--claim", "agent-2"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("agent-1"));
    Ok(())
}

#[test]
fn same_agent_reclaim_updates_claimed_at() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let first = project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let first_at = first["claimed_at"].as_str().unwrap().to_string();

    // Small delay to ensure different timestamp
    std::thread::sleep(std::time::Duration::from_millis(10));

    // Same agent re-claims — should succeed, updating claimed_at
    let second = project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    assert_eq!(second["status"], "doing");
    assert_eq!(second["claimed_by"], "agent-1");

    // claimed_at should have been updated
    let second_at = second["claimed_at"].as_str().unwrap();
    assert_ne!(
        second_at, first_at,
        "claimed_at should be updated on re-claim"
    );
    Ok(())
}

#[test]
fn claim_cleared_on_done() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let result = project.run_tg_json(&["done", id]);
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn claim_cleared_on_todo() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let result = project.run_tg_json(&["todo", id]);
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    Ok(())
}

#[test]
fn claim_visible_in_list() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);

    let list = project.run_tg_json(&["list"]);
    let items = list.as_array().unwrap();
    let item = items
        .iter()
        .find(|i| i["id"].as_str().unwrap() == id)
        .unwrap();
    assert_eq!(item["claimed_by"], "agent-1");
    assert!(!item["claimed_at"].is_null());
    Ok(())
}

// === Archival tests ===

#[test]
fn done_archives_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["done", id]);

    // Item should not be in active list
    let list = project.run_tg_json(&["list"]);
    assert!(list.as_array().unwrap().is_empty());

    // Item should be visible via show (archive fallback)
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["status"], "done");

    // Item should be in archive.jsonl directly
    let archive_path = project.project_dir().join("archive.jsonl");
    let archive_content = std::fs::read_to_string(archive_path)?;
    assert!(archive_content.contains(id));

    // Item should appear in list --status done
    let done_list = project.run_tg_json(&["list", "--status", "done"]);
    let done_items = done_list.as_array().unwrap();
    assert_eq!(done_items.len(), 1);
    assert_eq!(done_items[0]["id"].as_str().unwrap(), id);

    Ok(())
}

#[test]
fn done_terminal_state() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["done", id]);

    // Cannot transition further — item is archived, not in active store
    let output = project.run_tg(&["--json", "do", id]);
    assert!(!output.status.success());
    Ok(())
}

// === Dedicated tg todo test ===

#[test]
fn todo_unclaims_and_resets() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let before = project.run_tg_json(&["show", id]);
    let before_updated = before["updated_at"].as_str().unwrap().to_string();

    std::thread::sleep(std::time::Duration::from_millis(10));

    project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    let result = project.run_tg_json(&["todo", id]);
    assert_eq!(result["status"], "todo");
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());
    assert_ne!(result["updated_at"].as_str().unwrap(), before_updated);
    Ok(())
}

// === Dedicated todo→done test (skip doing) ===

#[test]
fn todo_directly_to_done() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    let result = project.run_tg_json(&["done", id]);
    assert_eq!(result["status"], "done");
    assert!(result["claimed_by"].is_null());
    assert!(result["claimed_at"].is_null());

    // Verify in archive
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["status"], "done");
    Ok(())
}

// === Unblock fallback test ===

#[test]
fn unblock_fallback_to_todo_when_blocked_from_missing() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // Manually edit the JSONL to set blocked status with null blocked_from_status
    let tasks_path = project.project_dir().join("tasks.jsonl");
    let content = std::fs::read_to_string(&tasks_path)?;
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Find and modify the item line
    for line in lines.iter_mut().skip(1) {
        if line.contains(id) {
            let mut item: serde_json::Value = serde_json::from_str(line)?;
            item["status"] = serde_json::json!("blocked");
            item["blocked_from_status"] = serde_json::Value::Null;
            *line = serde_json::to_string(&item)?;
        }
    }
    std::fs::write(&tasks_path, lines.join("\n") + "\n")?;

    // Unblock should default to todo
    let result = project.run_tg_json(&["unblock", id]);
    assert_eq!(result["status"], "todo");
    Ok(())
}

// === Archive truncated line recovery test ===

#[test]
fn archive_truncated_line_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add and complete an item
    let added1 = project.run_tg_json(&["add", "Item 1"]);
    let id1 = added1["id"].as_str().unwrap();
    project.run_tg_json(&["done", id1]);

    // Truncate the archive mid-line to simulate crash
    let archive_path = project.project_dir().join("archive.jsonl");
    let content = std::fs::read_to_string(&archive_path)?;
    let truncated = format!("{}{}", content, r#"{"id":"tg-trunc","title":"truncat"#);
    std::fs::write(&archive_path, truncated)?;

    // Add and complete another item — should succeed despite truncated line
    let added2 = project.run_tg_json(&["add", "Item 2"]);
    let id2 = added2["id"].as_str().unwrap();
    let result = project.run_tg_json(&["done", id2]);
    assert_eq!(result["status"], "done");

    // Verify both valid items are visible
    let shown1 = project.run_tg_json(&["show", id1]);
    assert_eq!(shown1["status"], "done");
    let shown2 = project.run_tg_json(&["show", id2]);
    assert_eq!(shown2["status"], "done");

    Ok(())
}

// === Full agent workflow ===

#[test]
fn full_agent_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Create item
    let added = project.run_tg_json(&["add", "Agent task"]);
    let id = added["id"].as_str().unwrap();

    // Claim and start
    let doing = project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    assert_eq!(doing["status"], "doing");
    assert_eq!(doing["claimed_by"], "agent-1");

    // Complete
    let done = project.run_tg_json(&["done", id]);
    assert_eq!(done["status"], "done");
    assert!(done["claimed_by"].is_null());

    // Verify archived
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["status"], "done");

    Ok(())
}

// === Per-command JSON schema validation ===

#[test]
fn json_schema_validation_transitions() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Schema test"]);
    let id = added["id"].as_str().unwrap();

    // Test `do` output schema
    let result = project.run_tg_json(&["do", id, "--claim", "agent-1"]);
    validate_item_schema(&result);
    assert_eq!(result["status"], "doing");

    // Test `block` output schema
    let result = project.run_tg_json(&["block", id, "--reason", "testing"]);
    validate_item_schema(&result);
    assert_eq!(result["status"], "blocked");

    // Test `unblock` output schema
    let result = project.run_tg_json(&["unblock", id]);
    validate_item_schema(&result);
    // Unblock restored to doing (was blocked from doing state)
    assert_eq!(result["status"], "doing");

    // Test `todo` output schema (doing → todo)
    let result = project.run_tg_json(&["todo", id]);
    validate_item_schema(&result);
    assert_eq!(result["status"], "todo");

    // Test `done` output schema (todo → done)
    let result = project.run_tg_json(&["done", id]);
    validate_item_schema(&result);
    assert_eq!(result["status"], "done");

    Ok(())
}

fn validate_item_schema(item: &serde_json::Value) {
    // Required fields
    assert!(item["id"].is_string(), "id must be string");
    assert!(item["title"].is_string(), "title must be string");
    assert!(item["status"].is_string(), "status must be string");
    assert!(item["priority"].is_number(), "priority must be number");
    assert!(item["tags"].is_array(), "tags must be array");
    assert!(
        item["dependencies"].is_array(),
        "dependencies must be array"
    );
    assert!(item["created_at"].is_string(), "created_at must be string");
    assert!(item["updated_at"].is_string(), "updated_at must be string");

    // Nullable fields must be present (not omitted)
    assert!(
        item.get("description").is_some(),
        "description must be present"
    );
    assert!(
        item.get("blocked_reason").is_some(),
        "blocked_reason must be present"
    );
    assert!(
        item.get("blocked_from_status").is_some(),
        "blocked_from_status must be present"
    );
    assert!(
        item.get("claimed_by").is_some(),
        "claimed_by must be present"
    );
    assert!(
        item.get("claimed_at").is_some(),
        "claimed_at must be present"
    );

    // ID format
    let id = item["id"].as_str().unwrap();
    assert!(
        id.starts_with("tg-") && id.len() == 8,
        "ID must match tg-XXXXX format: {}",
        id
    );

    // Status is valid enum
    let status = item["status"].as_str().unwrap();
    assert!(
        ["todo", "doing", "done", "blocked"].contains(&status),
        "Invalid status: {}",
        status
    );

    // Timestamps parse as ISO 8601
    chrono::DateTime::parse_from_rfc3339(item["created_at"].as_str().unwrap())
        .expect("created_at must be ISO 8601");
    chrono::DateTime::parse_from_rfc3339(item["updated_at"].as_str().unwrap())
        .expect("updated_at must be ISO 8601");
}
