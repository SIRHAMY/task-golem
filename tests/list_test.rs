mod common;

use common::TestProject;

#[test]
fn list_empty() {
    let proj = TestProject::new().unwrap();
    let json = proj.run_tg_json(&["list"]);
    assert_eq!(json, serde_json::json!([]));
}

#[test]
fn list_default_shows_non_done() {
    let proj = TestProject::new().unwrap();
    proj.run_tg_json(&["add", "Task A"]);
    proj.run_tg_json(&["add", "Task B"]);

    let json = proj.run_tg_json(&["list"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn list_filter_by_status() {
    let proj = TestProject::new().unwrap();
    proj.run_tg_json(&["add", "Task A"]);
    proj.run_tg_json(&["add", "Task B"]);

    // Both should be todo
    let json = proj.run_tg_json(&["list", "--status", "todo"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // No doing items
    let json = proj.run_tg_json(&["list", "--status", "doing"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}

#[test]
fn list_filter_by_tag() {
    let proj = TestProject::new().unwrap();
    proj.run_tg_json(&["add", "Task A", "--tag", "backend"]);
    proj.run_tg_json(&["add", "Task B", "--tag", "frontend"]);
    proj.run_tg_json(&["add", "Task C", "--tag", "backend"]);

    let json = proj.run_tg_json(&["list", "--tag", "backend"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn list_combined_filters() {
    let proj = TestProject::new().unwrap();
    proj.run_tg_json(&["add", "Task A", "--tag", "backend"]);
    proj.run_tg_json(&["add", "Task B", "--tag", "frontend"]);

    let json = proj.run_tg_json(&["list", "--status", "todo", "--tag", "backend"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "Task A");
}

#[test]
fn list_sort_order() {
    let proj = TestProject::new().unwrap();
    proj.run_tg_json(&["add", "Low priority", "--priority", "1"]);
    proj.run_tg_json(&["add", "High priority", "--priority", "10"]);
    proj.run_tg_json(&["add", "Medium priority", "--priority", "5"]);

    let json = proj.run_tg_json(&["list"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr[0]["title"], "High priority");
    assert_eq!(arr[1]["title"], "Medium priority");
    assert_eq!(arr[2]["title"], "Low priority");
}

#[test]
fn list_sort_order_priority_then_created() {
    let proj = TestProject::new().unwrap();
    // Add two items with same priority
    proj.run_tg_json(&["add", "First created"]);
    proj.run_tg_json(&["add", "Second created"]);

    let json = proj.run_tg_json(&["list"]);
    let arr = json.as_array().unwrap();
    // Same priority, so sorted by created_at asc
    assert_eq!(arr[0]["title"], "First created");
    assert_eq!(arr[1]["title"], "Second created");
}

#[test]
fn list_status_done_loads_archive() {
    let proj = TestProject::new().unwrap();
    // The archive is empty at start, so done filter should return empty
    let json = proj.run_tg_json(&["list", "--status", "done"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 0);
}

#[test]
fn list_invalid_status() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["--json", "list", "--status", "invalid"]);
    assert!(!output.status.success());
}

#[test]
fn list_parent_filter_returns_direct_children() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Parent"]);
    let pid = p["id"].as_str().unwrap().to_string();
    proj.run_tg_json(&["add", "Child A", "--parent", &pid]);
    proj.run_tg_json(&["add", "Child B", "--parent", &pid]);
    proj.run_tg_json(&["add", "Unrelated"]);

    let json = proj.run_tg_json(&["list", "--parent", &pid]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let titles: Vec<&str> = arr.iter().map(|v| v["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"Child A"));
    assert!(titles.contains(&"Child B"));
}

#[test]
fn list_parent_filter_empty_result() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Leaf"]);
    let pid = p["id"].as_str().unwrap().to_string();

    let json = proj.run_tg_json(&["list", "--parent", &pid]);
    let arr = json.as_array().unwrap();
    assert!(arr.is_empty());
}

#[test]
fn list_blocked_flag_matches_status_blocked() {
    let proj = TestProject::new().unwrap();
    let a = proj.run_tg_json(&["add", "Task A"]);
    let aid = a["id"].as_str().unwrap().to_string();
    let b = proj.run_tg_json(&["add", "Task B"]);
    let bid = b["id"].as_str().unwrap().to_string();

    proj.run_tg(&["block", &aid, "--reason", "stuck"]);

    let flag = proj.run_tg_json(&["list", "--blocked"]);
    let status = proj.run_tg_json(&["list", "--status", "blocked"]);

    // Same set (may be ordered identically — list sort is stable on
    // priority+created_at), assert JSON equality.
    assert_eq!(flag, status);

    let arr = flag.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], aid);
    // Sanity: unrelated non-blocked task not present.
    assert!(arr.iter().all(|v| v["id"] != bid));
}

#[test]
fn list_blocked_and_status_combined_errors() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["--json", "list", "--blocked", "--status", "todo"]);
    assert!(
        !output.status.success(),
        "expected InvalidInput error, got success"
    );
}

#[test]
fn list_parent_filter_combined_with_status() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Parent"]);
    let pid = p["id"].as_str().unwrap().to_string();
    let a = proj.run_tg_json(&["add", "Child A", "--parent", &pid]);
    let aid = a["id"].as_str().unwrap().to_string();
    proj.run_tg_json(&["add", "Child B", "--parent", &pid]);

    // Move A to doing
    proj.run_tg_json(&["do", &aid]);

    let json = proj.run_tg_json(&["list", "--parent", &pid, "--status", "doing"]);
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"].as_str().unwrap(), aid);
}
