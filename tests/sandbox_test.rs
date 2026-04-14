//! Integration tests for the `tg query` SELECT-only sandbox.
//!
//! Each test drives one denied action code end-to-end via the CLI, asserting
//! that the command exits non-zero and prints a `Query denied by sandbox`
//! message. The positive test at the bottom confirms that `sqlite_master`
//! reads still work — SELECT introspection is explicitly allowed.

mod common;

use common::TestProject;

/// Run `tg query <sql>` and return the exit code + stderr (for matching error
/// messages). We capture stderr because sandbox denials surface as plain-text
/// errors by default; `--json` would route them to stderr too.
fn run_query(project: &TestProject, sql: &str) -> (i32, String, String) {
    let out = project.run_tg(&["query", sql]);
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (code, stdout, stderr)
}

fn assert_denied(project: &TestProject, sql: &str) {
    let (code, _stdout, stderr) = run_query(project, sql);
    assert_ne!(code, 0, "expected nonzero exit for SQL: {}", sql);
    assert!(
        stderr.contains("Query denied by sandbox"),
        "expected sandbox denial for SQL `{}`, stderr was: {}",
        sql,
        stderr
    );
}

fn seed_project() -> TestProject {
    let project = TestProject::new().unwrap();
    // Seed a couple rows so the cache has something to read.
    project.run_tg(&["add", "first"]);
    project.run_tg(&["add", "second"]);
    project
}

// --- Denials: DML ---------------------------------------------------------

#[test]
fn sandbox_denies_insert() {
    let project = seed_project();
    assert_denied(
        &project,
        "INSERT INTO tasks(id, title, status, priority, created_at, updated_at) VALUES ('x','y','todo',0,'2026','2026')",
    );
}

#[test]
fn sandbox_denies_update() {
    let project = seed_project();
    assert_denied(&project, "UPDATE tasks SET title='x' WHERE id='nope'");
}

#[test]
fn sandbox_denies_delete() {
    let project = seed_project();
    assert_denied(&project, "DELETE FROM tasks");
}

// --- Denials: DDL ---------------------------------------------------------

#[test]
fn sandbox_denies_create_table() {
    let project = seed_project();
    assert_denied(&project, "CREATE TABLE evil(x TEXT)");
}

#[test]
fn sandbox_denies_create_virtual_table() {
    // Virtual-table modules have historically been a sandbox escape route
    // (fts3_tokenizer, etc.). The authorizer must block `CREATE VTABLE`.
    let project = seed_project();
    assert_denied(&project, "CREATE VIRTUAL TABLE evil USING fts5(content)");
}

#[test]
fn sandbox_denies_drop_table() {
    let project = seed_project();
    assert_denied(&project, "DROP TABLE tasks");
}

#[test]
fn sandbox_denies_alter_table() {
    let project = seed_project();
    assert_denied(&project, "ALTER TABLE tasks ADD COLUMN evil TEXT");
}

// --- Denials: attach ------------------------------------------------------

#[test]
fn sandbox_denies_attach() {
    let project = seed_project();
    assert_denied(&project, "ATTACH DATABASE '/tmp/evil.db' AS evil");
}

#[test]
fn sandbox_denies_detach() {
    // DETACH is denied even though ATTACH can never have succeeded — both
    // ops are off-limits regardless of prior state.
    let project = seed_project();
    assert_denied(&project, "DETACH DATABASE evil");
}

// --- Denials: pragma ------------------------------------------------------

#[test]
fn sandbox_denies_writable_schema() {
    let project = seed_project();
    assert_denied(&project, "PRAGMA writable_schema = ON");
}

#[test]
fn sandbox_denies_query_only_off() {
    // Escape attempt: flip back off the `query_only=ON` we set at connection
    // open. The allowlist PRAGMA rule rejects any assignment form, so this
    // must be denied.
    let project = seed_project();
    assert_denied(&project, "PRAGMA query_only = OFF");
}

// --- Denials: functions ---------------------------------------------------

#[test]
fn sandbox_denies_load_extension() {
    let project = seed_project();
    assert_denied(&project, "SELECT load_extension('/tmp/evil.so')");
}

#[test]
fn sandbox_denies_readfile() {
    // `readfile` is shell-tooling only (not linked into rusqlite's bundled
    // build), so the authorizer may never see it; in that case the query
    // fails as QuerySyntax. Either outcome blocks the attack — accept both.
    let project = TestProject::new().unwrap();
    project.run_tg(&["add", "first"]);
    let out = project.run_tg(&["query", "SELECT readfile('/etc/passwd')"]);
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(code, 0, "readfile must fail");
    assert!(
        stderr.contains("Query denied by sandbox") || stderr.contains("Query syntax error"),
        "expected denial or syntax error for readfile; got: {}",
        stderr
    );
}

// --- Denials: transactions ------------------------------------------------

#[test]
fn sandbox_denies_begin_commit() {
    let project = seed_project();
    // Multi-statement: `prepare` only handles the first, so send them
    // separately. BEGIN alone must be denied.
    assert_denied(&project, "BEGIN");
}

#[test]
fn sandbox_denies_reindex() {
    let project = seed_project();
    assert_denied(&project, "REINDEX");
}

#[test]
fn sandbox_denies_analyze() {
    let project = seed_project();
    assert_denied(&project, "ANALYZE");
}

// --- Denials: obfuscated ---------------------------------------------------

#[test]
fn sandbox_denies_obfuscated_delete_with_cte() {
    // CTEs that wrap a mutation still fire the authorizer on the nested
    // action code — the sandbox catches the hidden DELETE regardless of the
    // outer WITH wrapper. (Note: INSERT-inside-CTE isn't standard SQLite
    // syntax, so we use DELETE-with-CTE which the parser accepts.)
    let project = seed_project();
    assert_denied(
        &project,
        "WITH x AS (SELECT id FROM tasks) DELETE FROM tasks WHERE id IN (SELECT id FROM x)",
    );
}

// --- Positive test: sqlite_master reads allowed ---------------------------

#[test]
fn sandbox_allows_sqlite_master_reads() {
    let project = seed_project();
    let out = project.run_tg(&[
        "query",
        "SELECT name, type FROM sqlite_master WHERE type='table' ORDER BY name",
    ]);
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        code, 0,
        "sqlite_master SELECT must succeed; stderr={}",
        stderr
    );
    // We should see several table names we know we create.
    assert!(
        stdout.contains("tasks") && stdout.contains("task_view"),
        "expected tasks + task_view tables in sqlite_master output; got: {}",
        stdout
    );
}
