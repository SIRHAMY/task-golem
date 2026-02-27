mod common;

use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use common::TestProject;

/// Spawn two processes both trying to `tg do <id> --claim <agent>`.
/// Exactly one should succeed and one should fail.
#[test]
fn concurrent_claim_race() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Race item"]);
    let id = added["id"].as_str().unwrap().to_string();

    let path = project.path().to_path_buf();

    // Spawn both processes
    let child1 = Command::new(cargo_bin("tg"))
        .current_dir(&path)
        .args(["--json", "do", &id, "--claim", "agent-1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let child2 = Command::new(cargo_bin("tg"))
        .current_dir(&path)
        .args(["--json", "do", &id, "--claim", "agent-2"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let output1 = child1.wait_with_output()?;
    let output2 = child2.wait_with_output()?;

    let s1 = output1.status.success();
    let s2 = output2.status.success();

    // Exactly one should succeed
    assert!(
        (s1 && !s2) || (!s1 && s2),
        "Expected exactly one success: agent-1={}, agent-2={}",
        s1,
        s2
    );

    // Verify the item is in doing state with exactly one claim
    let shown = project.run_tg_json(&["show", &id]);
    assert_eq!(shown["status"], "doing");
    assert!(shown["claimed_by"].is_string());
    let claimer = shown["claimed_by"].as_str().unwrap();
    assert!(claimer == "agent-1" || claimer == "agent-2");

    Ok(())
}

/// Spawn 5 processes each adding a different item.
/// All 5 should succeed with no corruption.
#[test]
fn concurrent_adds() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let path = project.path().to_path_buf();

    let mut children = Vec::new();
    for i in 0..5 {
        let child = Command::new(cargo_bin("tg"))
            .current_dir(&path)
            .args(["--json", "add", &format!("Concurrent task {}", i)])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        children.push(child);
    }

    let mut successes = 0;
    for child in children {
        let output = child.wait_with_output()?;
        if output.status.success() {
            successes += 1;
        }
    }

    assert_eq!(successes, 5, "All 5 concurrent adds should succeed");

    // Verify all 5 items present
    let list = project.run_tg_json(&["list"]);
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 5);

    // All IDs should be unique
    let ids: std::collections::HashSet<&str> =
        items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 5, "All IDs should be unique");

    Ok(())
}
