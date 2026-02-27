mod common;

use common::TestProject;

#[test]
fn idempotent_done_on_archived_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // First done — normal transition
    let output = project.run_tg(&["--json", "done", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    // Normal done returns the item
    assert!(json["id"].is_string());
    assert_eq!(json["status"], "done");

    // Second done — idempotent (item already in archive)
    let output = project.run_tg(&["--json", "done", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["idempotent"], true);
    assert_eq!(json["previous_state"], "done");

    Ok(())
}

#[test]
fn idempotent_todo_on_todo_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // Item starts as todo — calling todo again should be idempotent
    let output = project.run_tg(&["--json", "todo", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["idempotent"], true);
    assert_eq!(json["previous_state"], "todo");

    Ok(())
}

#[test]
fn idempotent_do_on_doing_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["do", id]);

    // Calling do again without claim should be idempotent
    let output = project.run_tg(&["--json", "do", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["idempotent"], true);
    assert_eq!(json["previous_state"], "doing");

    Ok(())
}

#[test]
fn idempotent_block_on_blocked_item() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    project.run_tg_json(&["block", id]);

    // Calling block again should be idempotent
    let output = project.run_tg(&["--json", "block", id]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
    assert_eq!(json["idempotent"], true);
    assert_eq!(json["previous_state"], "blocked");

    Ok(())
}

#[test]
fn idempotent_do_claim_conflict_still_fails() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // Agent-1 claims
    project.run_tg(&["do", id, "--claim", "agent-1"]);

    // Agent-2 tries to claim — should still fail (not idempotent, it's a conflict)
    let output = project.run_tg(&["--json", "do", id, "--claim", "agent-2"]);
    assert!(!output.status.success());

    Ok(())
}

#[test]
fn idempotent_human_output() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Test item"]);
    let id = added["id"].as_str().unwrap();

    // todo on todo item — human mode
    let output = project.run_tg(&["todo", id]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Already todo"));

    Ok(())
}
