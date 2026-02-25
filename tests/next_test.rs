mod common;

use common::TestProject;

#[test]
fn next_returns_highest_priority_ready_item() {
    let project = TestProject::new().unwrap();

    // Add items with different priorities
    project.run_tg(&["add", "Low prio", "--priority", "1"]);
    project.run_tg(&["add", "High prio", "--priority", "10"]);
    project.run_tg(&["add", "Med prio", "--priority", "5"]);

    let next = project.run_tg_json(&["next"]);
    assert_eq!(next["priority"], 10);
    assert_eq!(next["title"], "High prio");

    // Should match first element of ready queue
    let ready = project.run_tg_json(&["ready"]);
    let first = &ready[0];
    assert_eq!(next["id"], first["id"]);
}

#[test]
fn next_returns_null_when_queue_empty() {
    let project = TestProject::new().unwrap();

    let next = project.run_tg_json(&["next"]);
    assert!(next.is_null(), "Expected null, got: {}", next);
}

#[test]
fn next_returns_null_when_all_items_blocked_or_doing() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    project.run_tg(&["do", a_id]);

    let next = project.run_tg_json(&["next"]);
    assert!(next.is_null(), "Expected null when no todo items, got: {}", next);
}

#[test]
fn next_human_output_when_empty() {
    let project = TestProject::new().unwrap();
    let output = project.run_tg(&["next"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No items ready"),
        "Expected 'No items ready', got: {}",
        stdout
    );
}
