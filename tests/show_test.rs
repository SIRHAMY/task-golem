mod common;

use common::TestProject;

#[test]
fn show_full_id() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();

    let show_json = proj.run_tg_json(&["show", id]);
    assert_eq!(show_json["id"], id);
    assert_eq!(show_json["title"], "Show me");
}

#[test]
fn show_bare_hex() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();
    let bare_hex = &id[3..]; // strip "tg-"

    let show_json = proj.run_tg_json(&["show", bare_hex]);
    assert_eq!(show_json["id"], id);
}

#[test]
fn show_prefix_match() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Show me"]);
    let id = add_json["id"].as_str().unwrap();
    let prefix = &id[3..6]; // first 3 hex chars

    let show_json = proj.run_tg_json(&["show", prefix]);
    assert_eq!(show_json["id"], id);
}

#[test]
fn show_not_found() {
    let proj = TestProject::new().unwrap();
    let output = proj.run_tg(&["show", "tg-zzz99"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn show_json_schema_complete() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Complete item", "--set", "x-test=true"]);
    let id = add_json["id"].as_str().unwrap();

    let show_json = proj.run_tg_json(&["show", id]);
    // All fields present including null fields
    assert!(show_json["id"].is_string());
    assert!(show_json["title"].is_string());
    assert!(show_json["status"].is_string());
    assert!(show_json["priority"].is_number());
    assert!(show_json.get("description").is_some()); // null but present
    assert!(show_json["tags"].is_array());
    assert!(show_json["dependencies"].is_array());
    assert!(show_json["created_at"].is_string());
    assert!(show_json["updated_at"].is_string());
    assert!(show_json.get("blocked_reason").is_some());
    assert!(show_json.get("blocked_from_status").is_some());
    assert!(show_json.get("claimed_by").is_some());
    assert!(show_json.get("claimed_at").is_some());
    assert_eq!(show_json["x-test"], true);
}

#[test]
fn show_children_section_rendered_for_parent() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Epic"]);
    let pid = p["id"].as_str().unwrap().to_string();
    proj.run_tg_json(&["add", "Sub A", "--parent", &pid]);
    proj.run_tg_json(&["add", "Sub B", "--parent", &pid, "--priority", "5"]);

    let output = proj.run_tg(&["show", &pid]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Children:"));
    // Higher-priority child listed first.
    let sub_a_pos = stdout.find("Sub A").unwrap();
    let sub_b_pos = stdout.find("Sub B").unwrap();
    assert!(sub_b_pos < sub_a_pos);
}

#[test]
fn show_children_section_truncates_past_10() {
    let proj = TestProject::new().unwrap();
    let p = proj.run_tg_json(&["add", "Big Epic"]);
    let pid = p["id"].as_str().unwrap().to_string();
    for i in 0..12 {
        proj.run_tg_json(&["add", &format!("Child {}", i), "--parent", &pid]);
    }

    let output = proj.run_tg(&["show", &pid]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Children:"));
    assert!(stdout.contains("(2 more)"));
}

#[test]
fn show_without_events_flag_omits_event_log() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task A"]);
    let id = add_json["id"].as_str().unwrap();
    proj.run_tg(&["note", id, "first note"]);

    let output = proj.run_tg(&["show", id]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The event log header "TIMESTAMP" should not appear without --events.
    assert!(
        !stdout.contains("TIMESTAMP"),
        "default show must not include event log, got: {}",
        stdout
    );
    // Nor should the note text.
    assert!(
        !stdout.contains("first note"),
        "default show must not include note text, got: {}",
        stdout
    );
}

#[test]
fn show_with_events_flag_appends_event_log() {
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task A"]);
    let id = add_json["id"].as_str().unwrap();
    proj.run_tg(&["note", id, "a specific note"]);

    let output = proj.run_tg(&["show", id, "--events"]);
    assert!(
        output.status.success(),
        "show --events should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("TIMESTAMP"),
        "show --events must include event log header, got: {}",
        stdout
    );
    assert!(
        stdout.contains("a specific note"),
        "show --events must include note text, got: {}",
        stdout
    );
}

#[test]
fn show_events_section_matches_tg_events_output() {
    // The events section in `show --events` reuses `print_events_human`, so
    // the event rows should match `tg events <id>` byte-for-byte. We can't
    // easily compare the detail view too, so just assert the events table
    // portion of `show --events` contains the exact output of `tg events`.
    let proj = TestProject::new().unwrap();
    let add_json = proj.run_tg_json(&["add", "Task A"]);
    let id = add_json["id"].as_str().unwrap();
    proj.run_tg(&["note", id, "note-one"]);
    proj.run_tg(&["note", id, "note-two"]);

    let events_out = proj.run_tg(&["events", id]);
    let events_stdout = String::from_utf8_lossy(&events_out.stdout).into_owned();
    let show_out = proj.run_tg(&["show", id, "--events"]);
    let show_stdout = String::from_utf8_lossy(&show_out.stdout);

    assert!(
        show_stdout.contains(events_stdout.trim_end()),
        "show --events must contain the events table verbatim.\nshow stdout:\n{}\nevents stdout:\n{}",
        show_stdout,
        events_stdout
    );
}

#[test]
fn show_archive_fallback() {
    let proj = TestProject::new().unwrap();
    // Manually write an item to the archive for testing
    let archive_path = proj.project_dir().join("archive.jsonl");
    let archive_content = r#"{"schema_version":1}
{"id":"tg-arc01","title":"Archived item","status":"done","priority":0,"description":null,"tags":[],"dependencies":[],"created_at":"2026-02-24T12:00:00Z","updated_at":"2026-02-24T12:00:00Z","blocked_reason":null,"blocked_from_status":null,"claimed_by":null,"claimed_at":null}
"#;
    std::fs::write(&archive_path, archive_content).unwrap();

    let show_json = proj.run_tg_json(&["show", "tg-arc01"]);
    assert_eq!(show_json["id"], "tg-arc01");
    assert_eq!(show_json["title"], "Archived item");
    assert_eq!(show_json["status"], "done");
}
