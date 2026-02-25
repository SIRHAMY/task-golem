mod common;

use common::TestProject;

#[test]
fn verbose_shows_diagnostics_on_stderr() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);

    let output = project.run_tg(&["--verbose", "list"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("[verbose]"),
        "Expected verbose diagnostics on stderr, got: {}",
        stderr
    );
    assert!(
        stderr.contains("Project root:"),
        "Expected project root diagnostic, got: {}",
        stderr
    );
}

#[test]
fn verbose_does_not_affect_json_stdout() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);

    let output = project.run_tg(&["--json", "--verbose", "list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // stdout should be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout should be valid JSON: {}\nstdout: {}", e, stdout));
    assert!(parsed.is_array());

    // stderr should have verbose output
    assert!(
        stderr.contains("[verbose]"),
        "Expected verbose on stderr: {}",
        stderr
    );
}

#[test]
fn verbose_on_ready_shows_item_counts() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);
    project.run_tg(&["add", "Task B"]);

    let output = project.run_tg(&["--verbose", "ready"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("active items"),
        "Expected active items count in verbose output, got: {}",
        stderr
    );
    assert!(
        stderr.contains("archive IDs"),
        "Expected archive IDs count in verbose output, got: {}",
        stderr
    );
}

#[test]
fn verbose_on_next_shows_diagnostics() {
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "Task A"]);

    let output = project.run_tg(&["--verbose", "next"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("[verbose]"),
        "Expected verbose diagnostics on stderr for next, got: {}",
        stderr
    );
}
