mod common;

use common::TestProject;

#[test]
fn no_color_env_disables_ansi_escapes() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);

    let output = project
        .tg_cmd()
        .env("NO_COLOR", "1")
        .arg("list")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // ANSI escape sequences start with \x1b[ or \033[
    assert!(
        !stdout.contains('\x1b'),
        "NO_COLOR=1 should produce no ANSI escapes, got: {:?}",
        stdout
    );
}

#[test]
fn force_color_enables_ansi_escapes() {
    let project = TestProject::new().unwrap();

    // Add items in different states to trigger colored output
    let a = project.run_tg_json(&["add", "Doing Task"]);
    let a_id = a["id"].as_str().unwrap();
    project.run_tg(&["do", a_id]);
    project.run_tg(&["add", "Todo Task"]);

    let output = project
        .tg_cmd()
        .env("FORCE_COLOR", "1")
        // Remove NO_COLOR if set
        .env_remove("NO_COLOR")
        .arg("list")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // When FORCE_COLOR is set, we expect ANSI escape sequences for colored status
    // The "doing" status should be yellow (ANSI yellow)
    assert!(
        stdout.contains('\x1b'),
        "FORCE_COLOR=1 should produce ANSI escapes for colored statuses, got: {:?}",
        stdout
    );
}

#[test]
fn json_output_never_contains_ansi() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);

    let output = project
        .tg_cmd()
        .env("FORCE_COLOR", "1")
        .args(["--json", "list"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "JSON output should never contain ANSI escapes: {:?}",
        stdout
    );
}
