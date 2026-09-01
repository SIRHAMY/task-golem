mod common;

use common::TestProject;

const ABSENT_ID: &str = "018f2b1c-4d5e-7abc-a345-6789abcdef01";
const LEGACY_ID: &str = "tg-a1b2c";

fn error_code(output: &std::process::Output) -> String {
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    error["code"].as_str().unwrap().to_string()
}

#[test]
fn canonical_id_supports_active_and_archive_lookup() {
    // Arrange
    let project = TestProject::new().unwrap();
    let created = project.run_tg_json(&["add", "Identity journey"]);
    let id = created["id"].as_str().unwrap();

    // Verify active lookup.
    let active = project.run_tg_json(&["show", id]);
    assert_eq!(active["id"], id);

    // Verify archive lookup.
    project.run_tg_json(&["done", id]);
    let archived = project.run_tg_json(&["show", id]);
    assert_eq!(archived["id"], id);
    assert_eq!(archived["status"], "done");
}

#[test]
fn cli_rejects_noncanonical_identity_forms_as_invalid_id() {
    // Arrange
    let project = TestProject::new().unwrap();
    let created = project.run_tg_json(&["add", "Identity validation"]);
    let id = created["id"].as_str().unwrap();
    let invalid_ids = [
        LEGACY_ID.to_string(),
        format!("tg-{id}"),
        id[..8].to_string(),
        id.replace('-', ""),
        id.to_ascii_uppercase(),
        "550e8400-e29b-41d4-a716-446655440000".to_string(),
        "not-a-uuid".to_string(),
    ];

    // Act and assert
    for invalid_id in invalid_ids {
        let output = project.run_tg(&["--json", "show", &invalid_id]);
        assert!(!output.status.success(), "{invalid_id} should be rejected");
        assert_eq!(error_code(&output), "invalid_id", "input: {invalid_id}");
    }
}

#[test]
fn item_level_id_inputs_share_exact_validation() {
    // Arrange
    let project = TestProject::new().unwrap();
    let created = project.run_tg_json(&["add", "Identity validation"]);
    let valid_id = created["id"].as_str().unwrap();
    let commands = [
        vec!["edit", LEGACY_ID, "--title", "updated"],
        vec!["rm", LEGACY_ID],
        vec!["do", LEGACY_ID, "--claim", "agent"],
        vec!["done", LEGACY_ID],
        vec!["todo", LEGACY_ID],
        vec!["block", LEGACY_ID],
        vec!["unblock", LEGACY_ID],
        vec!["note", LEGACY_ID, "note"],
        vec!["events", LEGACY_ID],
        vec!["list", "--parent", LEGACY_ID],
        vec!["add", "Child", "--parent", LEGACY_ID],
        vec!["add", "Dependent", "--dep", LEGACY_ID],
        vec!["dep", "add", valid_id, LEGACY_ID],
        vec!["dep", "rm", valid_id, LEGACY_ID],
    ];

    // Act and assert
    for command in commands {
        let mut args = vec!["--json"];
        args.extend(command);
        let output = project.run_tg(&args);
        assert!(!output.status.success(), "command should reject legacy ID");
        assert_eq!(error_code(&output), "invalid_id", "args: {args:?}");
    }

    let items = project.run_tg_json(&["list"]);
    assert_eq!(items.as_array().unwrap().len(), 1);
}

#[test]
fn valid_absent_id_is_item_not_found() {
    let project = TestProject::new().unwrap();

    let output = project.run_tg(&["--json", "show", ABSENT_ID]);

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "item_not_found");
}
