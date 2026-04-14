//! End-to-end tests for the `tg query` CLI.
//!
//! These tests exercise the happy paths (tabular + JSON output, recursive
//! CTEs, `--schema`, `--timeout`) through the actual binary so we catch
//! dispatch/formatting regressions. Sandbox denials have their own dedicated
//! test file (`sandbox_test.rs`).

mod common;

use common::TestProject;

/// Helper: `tg query <sql>` and return (exit, stdout, stderr).
fn run_query(project: &TestProject, args: &[&str]) -> (i32, String, String) {
    let out = project.run_tg(args);
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (code, stdout, stderr)
}

/// Add a task and return its ID from the created-line output.
fn add_task(project: &TestProject, title: &str, extra: &[&str]) -> String {
    let mut args = vec!["add", title];
    args.extend_from_slice(extra);
    let out = project.run_tg(&args);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let id_line = stdout
        .lines()
        .find(|l| l.contains("Created item:"))
        .unwrap_or_else(|| panic!("no Created item line in: {}", stdout));
    // "Created item: tg-xxxxx - title"
    id_line
        .split_whitespace()
        .nth(2)
        .unwrap_or_else(|| panic!("couldn't parse id from: {}", id_line))
        .to_string()
}

#[test]
fn query_count_tasks() {
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    add_task(&project, "two", &[]);
    let (code, stdout, stderr) = run_query(&project, &["query", "SELECT count(*) AS n FROM tasks"]);
    assert_eq!(code, 0, "stderr: {}", stderr);
    // Header + one data row; the count should be 2.
    assert!(stdout.contains("n"), "missing header: {}", stdout);
    assert!(stdout.contains("2"), "expected count=2: {}", stdout);
}

#[test]
fn query_json_envelope_shape() {
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    let (code, stdout, _) = run_query(
        &project,
        &["query", "SELECT count(*) AS n FROM tasks", "--json"],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parse");
    assert_eq!(v["columns"], serde_json::json!(["n"]));
    assert_eq!(v["rows"], serde_json::json!([[1]]));
}

#[test]
fn query_schema_prints_markdown() {
    let project = TestProject::new().unwrap();
    let (code, stdout, _) = run_query(&project, &["query", "--schema"]);
    assert_eq!(code, 0);
    // Markdown markers + key sections.
    assert!(
        stdout.contains("# task-golem cache schema"),
        "missing title heading: {}",
        stdout
    );
    assert!(stdout.contains("## DDL"), "missing DDL section");
    assert!(
        stdout.contains("## `task_view` columns"),
        "missing task_view columns section"
    );
    assert!(
        stdout.contains("active tasks only"),
        "missing active-only callout"
    );
    assert!(stdout.contains("depth < 64"), "missing depth < 64 reminder");
}

#[test]
fn query_timeout_zero_trips_immediately() {
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    let (code, _stdout, stderr) = run_query(
        &project,
        &["query", "SELECT * FROM tasks", "--timeout", "0"],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("Query exceeded timeout of 0s"),
        "expected timeout error: {}",
        stderr
    );
}

#[test]
fn query_recursive_descendants_cte() {
    let project = TestProject::new().unwrap();
    // Build a small tree: root -> a -> b, root -> c.
    let root = add_task(&project, "root", &[]);
    let a = add_task(&project, "a", &["--parent", &root]);
    let _b = add_task(&project, "b", &["--parent", &a]);
    let _c = add_task(&project, "c", &["--parent", &root]);

    let sql = format!(
        "WITH RECURSIVE d(id, depth) AS (
             SELECT id, 0 FROM tasks WHERE id = '{root}'
             UNION ALL
             SELECT t.id, d.depth + 1 FROM tasks t
               JOIN d ON t.parent = d.id
               WHERE d.depth < 64
         )
         SELECT count(*) AS n FROM d",
        root = root
    );
    let (code, stdout, stderr) = run_query(&project, &["query", &sql, "--json"]);
    assert_eq!(code, 0, "stderr: {}", stderr);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Root + a + b + c = 4.
    assert_eq!(v["rows"], serde_json::json!([[4]]));
}

#[test]
fn query_recursive_ancestors_cte() {
    let project = TestProject::new().unwrap();
    let root = add_task(&project, "root", &[]);
    let a = add_task(&project, "a", &["--parent", &root]);
    let b = add_task(&project, "b", &["--parent", &a]);

    let sql = format!(
        "WITH RECURSIVE a(id, parent, depth) AS (
             SELECT id, parent, 0 FROM tasks WHERE id = '{b}'
             UNION ALL
             SELECT t.id, t.parent, a.depth + 1 FROM tasks t
               JOIN a ON t.id = a.parent
               WHERE a.depth < 64
         )
         SELECT count(*) AS n FROM a",
        b = b
    );
    let (code, stdout, stderr) = run_query(&project, &["query", &sql, "--json"]);
    assert_eq!(code, 0, "stderr: {}", stderr);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // b + a + root = 3.
    assert_eq!(v["rows"], serde_json::json!([[3]]));
}

#[test]
fn query_task_view_is_ready() {
    let project = TestProject::new().unwrap();
    // Two roots: one with an unmet dep, one without.
    let a = add_task(&project, "a", &[]);
    let _b = add_task(&project, "b", &["--dep", &a]);
    let (code, stdout, stderr) = run_query(
        &project,
        &[
            "query",
            "SELECT count(*) AS n FROM task_view WHERE is_ready = 1",
            "--json",
        ],
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Only `a` is ready (no unmet deps); `b` has unmet dep on `a`.
    assert_eq!(v["rows"], serde_json::json!([[1]]));
}

#[test]
fn query_empty_result_shows_zero_rows_line() {
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    let (code, stdout, stderr) = run_query(
        &project,
        &["query", "SELECT id FROM tasks WHERE id = 'nonexistent'"],
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    assert!(
        stdout.contains("(0 rows)"),
        "expected (0 rows) in empty result: {}",
        stdout
    );
}

#[test]
fn query_json_empty_result() {
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    let (code, stdout, _) = run_query(
        &project,
        &[
            "query",
            "SELECT id FROM tasks WHERE id = 'nonexistent'",
            "--json",
        ],
    );
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["rows"], serde_json::json!([]));
    assert_eq!(v["columns"], serde_json::json!(["id"]));
}

#[test]
fn query_missing_sql_without_schema_is_error() {
    let project = TestProject::new().unwrap();
    let (code, _stdout, stderr) = run_query(&project, &["query"]);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("requires a SQL string") || stderr.contains("--schema"),
        "expected helpful error; got: {}",
        stderr
    );
}

#[test]
fn query_large_integer_emits_as_string_in_json() {
    // Ensure the JSON-safe-integer rule triggers for >2^53. We synthesize a
    // large value via SQL (bit-shift) rather than needing an actual stored
    // large integer.
    let project = TestProject::new().unwrap();
    add_task(&project, "one", &[]);
    let (code, stdout, stderr) = run_query(
        &project,
        &["query", "SELECT 9223372036854775807 AS big", "--json"],
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Should be a string to preserve precision.
    assert!(
        v["rows"][0][0].is_string(),
        "expected string-encoded large int: {:?}",
        v
    );
    assert!(
        stderr.contains("exceeds JSON-safe range"),
        "expected stderr warning; got: {}",
        stderr
    );
}
