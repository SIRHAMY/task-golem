mod common;

use std::fs;

use common::TestProject;

#[test]
fn archive_no_items_to_process() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["archive"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No items to archive or prune"));
    Ok(())
}

#[test]
fn archive_recovers_done_items_from_active() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add an item and manually set it to done in active store (without archiving)
    let added = project.run_tg_json(&["add", "Task to recover"]);
    let id = added["id"].as_str().unwrap().to_string();

    // Manually modify the tasks.jsonl to have a done item (simulate edge case)
    let tasks_path = project.path().join(".task-golem/tasks.jsonl");
    let content = fs::read_to_string(&tasks_path)?;
    let modified = content.replace("\"status\":\"todo\"", "\"status\":\"done\"");
    fs::write(&tasks_path, modified)?;

    // Run archive to recover
    let json = project.run_tg_json(&["archive"]);
    assert_eq!(json["recovered"], 1);
    assert!(json["recovered_ids"].as_array().unwrap().iter().any(|v| v.as_str().unwrap() == id));

    // Verify the item is now in archive and not in active
    let list = project.run_tg_json(&["list"]);
    let active_items = list.as_array().unwrap();
    assert!(active_items.is_empty());

    // Show the item from archive
    let shown = project.run_tg_json(&["show", &id]);
    assert_eq!(shown["status"], "done");

    Ok(())
}

#[test]
fn archive_prune_before_date() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Add and complete two items
    let added1 = project.run_tg_json(&["add", "Old task"]);
    let id1 = added1["id"].as_str().unwrap().to_string();
    project.run_tg_json(&["done", &id1]);

    let added2 = project.run_tg_json(&["add", "New task"]);
    let id2 = added2["id"].as_str().unwrap().to_string();
    project.run_tg_json(&["done", &id2]);

    // Manually set the first archived item's updated_at to an old date
    let archive_path = project.path().join(".task-golem/archive.jsonl");
    let content = fs::read_to_string(&archive_path)?;
    let mut new_lines = Vec::new();
    for line in content.lines() {
        if line.contains(&id1) {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            let old_date = parsed["updated_at"].as_str().unwrap().to_string();
            let modified_line =
                line.replace(&format!("\"updated_at\":\"{}\"", old_date), "\"updated_at\":\"2020-01-01T00:00:00Z\"");
            new_lines.push(modified_line);
        } else {
            new_lines.push(line.to_string());
        }
    }
    fs::write(&archive_path, new_lines.join("\n") + "\n")?;

    // Run archive with --before to prune old entries
    let json = project.run_tg_json(&["archive", "--before", "2025-01-01"]);
    assert_eq!(json["pruned"], 1);
    assert!(json["pruned_ids"].as_array().unwrap().iter().any(|v| v.as_str().unwrap() == id1));

    // Verify pruned items went to archive-pruned.jsonl
    let pruned_path = project.path().join(".task-golem/archive-pruned.jsonl");
    assert!(pruned_path.exists());
    let pruned_content = fs::read_to_string(&pruned_path)?;
    assert!(pruned_content.contains(&id1));

    // The remaining archive should only have the new item
    let list = project.run_tg_json(&["list", "--status", "done"]);
    let done_items = list.as_array().unwrap();
    assert_eq!(done_items.len(), 1);
    assert_eq!(done_items[0]["id"].as_str().unwrap(), id2);

    Ok(())
}

#[test]
fn archive_json_output_schema() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&["archive"]);
    assert_eq!(json["recovered"], 0);
    assert_eq!(json["pruned"], 0);
    assert!(json["recovered_ids"].is_array());
    assert!(json["pruned_ids"].is_array());

    Ok(())
}

#[test]
fn archive_invalid_date_format() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["archive", "--before", "not-a-date"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid date"));

    Ok(())
}
