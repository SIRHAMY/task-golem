//! `tg query` — run SELECT SQL against the cache.
//!
//! Dispatch shape mirrors `edit.rs`: resolve the store, route by flag set,
//! format output. The sandbox itself lives in `cache::query::execute`; this
//! module only handles arg parsing, cache freshness notices, and output
//! formatting (aligned-tabular or JSON envelope).

use std::io::{self, Write};
use std::time::{Duration, Instant};

use task_golem::cache;
use task_golem::cache::{QueryResult, SqlValue};
use task_golem::errors::TgError;
use task_golem::store::Store;
use task_golem::store::root;

/// Integers outside this range emit as JSON strings with a stderr warning —
/// JSON numbers are double-precision, and integers above 2^53 lose
/// precision on many JSON consumers. Keeps agents honest about overflow.
const JSON_SAFE_INT_MAX: i64 = 9_007_199_254_740_992; // 2^53
const JSON_SAFE_INT_MIN: i64 = -9_007_199_254_740_992;

pub fn run(
    verbose: bool,
    sql: Option<String>,
    schema: bool,
    json_mode: bool,
    timeout_secs: u64,
) -> Result<(), TgError> {
    if schema {
        return print_schema();
    }

    let sql = sql.ok_or_else(|| {
        TgError::InvalidInput("tg query requires a SQL string (or pass --schema)".to_string())
    })?;

    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    // Verbose rebuild notice: detect staleness up-front so we can both report
    // the rebuild starting and measure its duration. `execute` would do the
    // rebuild silently otherwise.
    if verbose && cache::is_stale(&store)? {
        let start = Instant::now();
        // Force the rebuild now (execute would do it again, but open_or_rebuild
        // is a no-op on the second call since the stamp now matches).
        let _ = cache::open_or_rebuild(&store, true)?;
        let elapsed = start.elapsed();
        eprintln!("rebuilding cache ({} ms)", elapsed.as_millis());
    }

    let timeout = Duration::from_secs(timeout_secs);
    let result = cache::query::execute(&store, &sql, timeout).map_err(|e| match e {
        // The sandbox module doesn't know the user-facing timeout value; patch
        // it in here so the error message matches what the user asked for.
        TgError::QueryTimeout { .. } => TgError::QueryTimeout {
            limit_secs: timeout_secs,
        },
        other => other,
    })?;

    if json_mode {
        print_json(&result);
    } else {
        print_tabular(&result);
    }
    Ok(())
}

/// Render the cache schema as Markdown suitable for agents reading it via
/// `tg query --schema`.
fn print_schema() -> Result<(), TgError> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let doc = format!(
        "# task-golem cache schema (v{version})\n\
         \n\
         > The cache is lazily rebuilt from `tasks.jsonl`. v1 contains **active tasks only** — archived tasks are not queryable via `tg query`.\n\
         > Bound any recursive CTE over `tasks.parent` with `WHERE depth < 64` to stay within the cache's materialized depth cap.\n\
         \n\
         ## DDL\n\
         \n\
         ```sql\n{ddl}```\n\
         \n\
         ## `task_view` columns\n\
         \n\
         The materialized agent-facing view. Most queries can use `task_view` directly without writing recursive CTEs.\n\
         \n\
         | Column | Type | Description |\n\
         | --- | --- | --- |\n\
         | `id` | TEXT | Task ID (primary key). |\n\
         | `title` | TEXT | Task title. |\n\
         | `status` | TEXT | One of `todo`, `doing`, `done`, `blocked`. |\n\
         | `priority` | INTEGER | Higher = more important. |\n\
         | `parent` | TEXT | Parent task ID, or NULL for roots. |\n\
         | `depth_from_root` | INTEGER | Distance from root (0 = root). Capped at 64. |\n\
         | `is_ready` | INTEGER | 1 iff `status='todo'` AND all dependencies are done. |\n\
         | `unmet_dep_count` | INTEGER | Number of dependencies not yet in `done`. |\n\
         \n\
         ## Notes\n\
         \n\
         - Only the `table_info`, `index_list`, `index_info` PRAGMAs are permitted.\n\
         - All mutations (INSERT/UPDATE/DELETE/DDL), ATTACH/DETACH, and file-access functions are denied by the sandbox.\n\
         - `sqlite_master` and `sqlite_schema` reads are permitted for introspection.\n",
        version = cache::SCHEMA_VERSION,
        ddl = cache::DDL,
    );

    writeln!(out, "{}", doc).ok();
    Ok(())
}

/// Aligned tabular output that mirrors the `tg list` width calculation:
/// measure the widest cell per column (including the header), then pad.
fn print_tabular(result: &QueryResult) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    if result.rows.is_empty() {
        // Still print the header so users see the column list.
        if !result.columns.is_empty() {
            let widths: Vec<usize> = result.columns.iter().map(|c| c.len()).collect();
            write_row(&mut out, &result.columns, &widths);
        }
        writeln!(out, "(0 rows)").ok();
        return;
    }

    // Compute column widths: max over (header, every cell).
    let mut widths: Vec<usize> = result.columns.iter().map(|c| c.len()).collect();
    let mut cell_strings: Vec<Vec<String>> = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let row_strs: Vec<String> = row.iter().map(cell_display).collect();
        for (i, s) in row_strs.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(s.len());
            }
        }
        cell_strings.push(row_strs);
    }

    write_row(&mut out, &result.columns, &widths);
    for row in &cell_strings {
        write_row(&mut out, row, &widths);
    }
}

fn write_row<W: Write>(out: &mut W, cells: &[String], widths: &[usize]) {
    let parts: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let w = widths.get(i).copied().unwrap_or(c.len());
            format!("{:<width$}", c, width = w)
        })
        .collect();
    writeln!(out, "{}", parts.join("  ")).ok();
}

fn cell_display(v: &SqlValue) -> String {
    match v {
        SqlValue::Null => String::new(),
        SqlValue::Integer(i) => i.to_string(),
        SqlValue::Real(f) => f.to_string(),
        SqlValue::Text(s) => s.clone(),
        SqlValue::Blob(_) => "<blob>".to_string(),
    }
}

/// JSON envelope: `{"columns": [...], "rows": [[...], ...]}`. Per-value
/// mapping follows the DESIGN §Error Handling table.
fn print_json(result: &QueryResult) {
    let mut warnings_emitted: bool = false;

    let rows_json: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|v| value_to_json(v, &mut warnings_emitted))
                .collect()
        })
        .collect();

    let envelope = serde_json::json!({
        "columns": result.columns,
        "rows": rows_json,
    });

    let stdout = io::stdout();
    let mut out = stdout.lock();
    writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| "{}".to_string())
    )
    .ok();
}

fn value_to_json(v: &SqlValue, warnings: &mut bool) -> serde_json::Value {
    match v {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => {
            if !(JSON_SAFE_INT_MIN..=JSON_SAFE_INT_MAX).contains(i) {
                if !*warnings {
                    eprintln!(
                        "warning: integer {} exceeds JSON-safe range (2^53); emitting as string",
                        i
                    );
                    *warnings = true;
                }
                serde_json::Value::String(i.to_string())
            } else {
                serde_json::Value::Number((*i).into())
            }
        }
        SqlValue::Real(f) => {
            if f.is_nan() || f.is_infinite() {
                eprintln!("warning: non-finite real value {} emitted as JSON null", f);
                serde_json::Value::Null
            } else {
                serde_json::Number::from_f64(*f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            }
        }
        SqlValue::Text(s) => serde_json::Value::String(s.clone()),
        SqlValue::Blob(_) => {
            eprintln!("warning: blob column encountered; emitting as null");
            serde_json::Value::Null
        }
    }
}
