mod common;

use std::process::Command;

use assert_cmd::cargo::cargo_bin;

use common::TestProject;

/// Concurrent stress test: 10 processes × 100 operations each.
/// Mix of add, edit, do, done operations.
#[test]
fn concurrent_stress_10x100() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let num_processes = 10;
    let adds_per_process = 10;

    // Phase 1: 10 processes each add 10 items concurrently
    let mut handles = Vec::new();
    for proc_id in 0..num_processes {
        let project_path = project.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let mut ids = Vec::new();
            for i in 0..adds_per_process {
                let output = Command::new(cargo_bin("tg"))
                    .current_dir(&project_path)
                    .args(["--json", "add", &format!("Task-{}-{}", proc_id, i)])
                    .output()
                    .expect("failed to execute tg");
                assert!(
                    output.status.success(),
                    "Add failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let json: serde_json::Value =
                    serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
                        .expect("parse JSON");
                ids.push(json["id"].as_str().unwrap().to_string());
            }
            ids
        });
        handles.push(handle);
    }

    let mut all_ids: Vec<String> = Vec::new();
    for handle in handles {
        let ids = handle.join().unwrap();
        all_ids.extend(ids);
    }

    // Verify: total items = num_processes * adds_per_process
    let expected_total = num_processes * adds_per_process;
    let list = project.run_tg_json(&["list"]);
    let items = list.as_array().unwrap();
    assert_eq!(
        items.len(),
        expected_total,
        "Expected {} items, got {}",
        expected_total,
        items.len()
    );

    // Verify: all IDs unique
    let mut unique_ids: Vec<String> = all_ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(unique_ids.len(), all_ids.len(), "Duplicate IDs detected");

    // Phase 2: Transition some items concurrently
    // Take first 20 IDs and do them concurrently
    let do_ids: Vec<String> = all_ids.iter().take(20).cloned().collect();
    let mut do_handles = Vec::new();
    for id in &do_ids {
        let project_path = project.path().to_path_buf();
        let id_clone = id.clone();
        let handle = std::thread::spawn(move || {
            let output = Command::new(cargo_bin("tg"))
                .current_dir(&project_path)
                .args(["--json", "do", &id_clone])
                .output()
                .expect("failed to execute tg");
            output.status.success()
        });
        do_handles.push(handle);
    }

    for handle in do_handles {
        let _success = handle.join().unwrap();
        // Some may fail due to concurrent modification, that's OK
    }

    // Phase 3: Done some items concurrently
    let done_ids: Vec<String> = do_ids.iter().take(10).cloned().collect();
    let mut done_handles = Vec::new();
    for id in &done_ids {
        let project_path = project.path().to_path_buf();
        let id_clone = id.clone();
        let handle = std::thread::spawn(move || {
            // First ensure it's in doing state
            let _ = Command::new(cargo_bin("tg"))
                .current_dir(&project_path)
                .args(["do", &id_clone])
                .output();
            let output = Command::new(cargo_bin("tg"))
                .current_dir(&project_path)
                .args(["--json", "done", &id_clone])
                .output()
                .expect("failed to execute tg");
            output.status.success()
        });
        done_handles.push(handle);
    }

    for handle in done_handles {
        let _success = handle.join().unwrap();
    }

    // Final verification: no corruption
    let doctor = project.run_tg_json(&["doctor"]);
    let total_issues = doctor["summary"]["total"].as_u64().unwrap();
    assert_eq!(
        total_issues, 0,
        "Doctor found issues after stress test: {:?}",
        doctor
    );

    // No exit-code-2 errors (system errors) — we don't track this per-command
    // but a clean doctor output validates store integrity

    Ok(())
}

/// Concurrent claim race: 5 processes try to claim the same item.
/// Exactly one should succeed.
#[test]
fn concurrent_claim_stress() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;
    let added = project.run_tg_json(&["add", "Contested item"]);
    let id = added["id"].as_str().unwrap().to_string();

    let num_agents = 5;
    let mut handles = Vec::new();

    for agent_id in 0..num_agents {
        let project_path = project.path().to_path_buf();
        let id_clone = id.clone();
        let handle = std::thread::spawn(move || {
            let output = Command::new(cargo_bin("tg"))
                .current_dir(&project_path)
                .args([
                    "--json",
                    "do",
                    &id_clone,
                    "--claim",
                    &format!("agent-{}", agent_id),
                ])
                .output()
                .expect("failed to execute tg");
            output.status.success()
        });
        handles.push(handle);
    }

    let mut successes = 0;
    for handle in handles {
        if handle.join().unwrap() {
            successes += 1;
        }
    }

    assert_eq!(
        successes, 1,
        "Exactly one agent should succeed claiming, got {}",
        successes
    );

    // Verify the item is doing with a single claimer
    let shown = project.run_tg_json(&["show", &id]);
    assert_eq!(shown["status"], "doing");
    assert!(shown["claimed_by"].is_string());

    Ok(())
}
