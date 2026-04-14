//! SELECT-only query execution with a three-layer sandbox.
//!
//! Security-critical module. The sandbox layers are:
//!
//! 1. **OS-level** — the query connection opens `cache.db` with
//!    `SQLITE_OPEN_READ_ONLY`, so no write can reach the filesystem via this
//!    handle regardless of SQL.
//! 2. **Engine-level** — `PRAGMA query_only=ON`, `PRAGMA trusted_schema=OFF`,
//!    and `PRAGMA defensive=ON` are applied before prepare. These disable the
//!    write path inside the engine and disarm a number of historical attack
//!    surfaces (schema-shadowing, `fts3_tokenizer` injection, etc.).
//! 3. **Statement-level** — an allowlist authorizer fires on every AST node.
//!    Default-deny: anything not explicitly listed is rejected at prepare
//!    time. New SQLite action codes introduced by upstream version bumps stay
//!    blocked automatically.
//!
//! Allowed actions (see DESIGN §Decision: Three-layer SELECT-only sandbox):
//! - `SQLITE_SELECT`, `SQLITE_READ`, `SQLITE_RECURSIVE`.
//! - `SQLITE_FUNCTION` *unless* the function name is in the denylist
//!   (`load_extension`, `readfile`, `writefile`, `edit`, `fts3_tokenizer`).
//! - `SQLITE_PRAGMA` *only* for `table_info`, `index_list`, `index_info`.
//! - `sqlite_master` / `sqlite_schema` reads are allowed (covered by the
//!   generic `SQLITE_READ` allowance) — they're pure reads and the authorizer
//!   still blocks every mutation path regardless.
//!
//! Per-query timeout is enforced via `progress_handler(1000, …)` with a
//! shared deadline; on trip, SQLite returns `SQLITE_INTERRUPT`, which we map
//! to [`TgError::QueryTimeout`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Type, ValueRef};
use rusqlite::{Connection, OpenFlags};

use super::{QueryResult, SqlType, SqlValue};
use crate::errors::TgError;
use crate::store::Store;

/// Functions whose execution is always denied, even though `SQLITE_FUNCTION`
/// in general is allowed. Kept lowercase; comparison is case-insensitive.
const DENIED_FUNCTIONS: &[&str] = &[
    "load_extension",
    "readfile",
    "writefile",
    "edit",
    "fts3_tokenizer",
];

/// Pragmas the sandbox permits. All three are purely introspective — no side
/// effects, no writes, no schema mutation.
const ALLOWED_PRAGMAS: &[&str] = &["table_info", "index_list", "index_info"];

/// Execute `sql` against the cache with the full sandbox in place.
///
/// `store` is used to resolve the cache path and to drive the lazy rebuild in
/// [`crate::cache::open_or_rebuild`]. The actual query connection is opened
/// separately with `SQLITE_OPEN_READ_ONLY` after the rebuild completes, so
/// any lock held by the rebuild path is already released by the time the
/// query begins executing.
pub fn execute(store: &Store, sql: &str, timeout: Duration) -> Result<QueryResult, TgError> {
    // Ensure the cache is fresh. The connection returned here is discarded
    // immediately; we re-open with stricter flags below.
    let _ = super::open_or_rebuild(store, false)?;

    let cache_path = store.cache_db_path();
    let conn = Connection::open_with_flags(&cache_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
        |e| TgError::CacheCorrupt {
            detail: format!("failed to open {} for query: {}", cache_path.display(), e),
        },
    )?;

    apply_pragmas(&conn)?;
    install_authorizer(&conn);
    install_progress_handler(&conn, timeout);

    // `--timeout 0` is documented as "trip immediately". Short-circuit here so
    // trivial queries like `SELECT 1` (which may complete before the progress
    // handler fires) still honor the contract.
    if timeout.is_zero() {
        return Err(TgError::QueryTimeout { limit_secs: 0 });
    }

    let result = run_query(&conn, sql);

    // Clear per-query state so nothing leaks if the connection ever did
    // persist (it doesn't today, but keep this defensive).
    conn.progress_handler(0, None::<fn() -> bool>);
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);

    result
}

/// Apply engine-level safety pragmas. Any failure here is a bug (these are
/// always-available in a bundled SQLite), but we surface it as a rebuild
/// error rather than panicking so `tg query` prints a useful message.
fn apply_pragmas(conn: &Connection) -> Result<(), TgError> {
    conn.execute_batch(
        "PRAGMA query_only = ON;\n\
         PRAGMA trusted_schema = OFF;\n\
         PRAGMA defensive = ON;",
    )
    .map_err(|e| TgError::CacheCorrupt {
        detail: format!("applying safety pragmas: {}", e),
    })
}

/// Install the default-deny allowlist authorizer.
fn install_authorizer(conn: &Connection) {
    conn.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        // Top-level SELECT and internal READ + recursive CTE — all allowed.
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }

        // Functions: allow unless explicitly denied by name.
        AuthAction::Function { function_name } => {
            if DENIED_FUNCTIONS
                .iter()
                .any(|n| n.eq_ignore_ascii_case(function_name))
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }

        // Pragmas: only the three introspective ones, read-only form (no value).
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => {
            let allowed = ALLOWED_PRAGMAS
                .iter()
                .any(|n| n.eq_ignore_ascii_case(pragma_name));
            // Deny any pragma assignment (`PRAGMA x = y`) even for allowlisted
            // names. The read form is enough for schema inspection and
            // blocks escapes like `PRAGMA query_only = OFF` if the name ever
            // slipped into the allowlist by accident.
            if allowed && pragma_value.is_none() {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }

        // Everything else — writes, schema changes, attach, transactions,
        // savepoints, future action codes — is denied.
        _ => Authorization::Deny,
    }));
}

/// Install a per-query progress handler that trips once wall-clock time
/// exceeds `deadline`. Granularity is 1000 VM-ops; at our statement sizes
/// that's sub-millisecond overhead.
///
/// The `timeout == 0` case is handled by [`execute`] before this runs (it
/// short-circuits with [`TgError::QueryTimeout`]), so here we only need to
/// handle "real" timeouts. If `Instant::now() + timeout` overflows (absurdly
/// large timeout), treat it as "never trips" — the user asked for essentially
/// no cap.
fn install_progress_handler(conn: &Connection, timeout: Duration) {
    let deadline = Instant::now().checked_add(timeout);
    // `tripped` latches the interrupt once fired so a second call (post-
    // rollback SQLite may invoke the handler again) keeps returning true.
    // `AtomicBool` because the rusqlite handler is `Send + 'static`.
    let tripped = Arc::new(AtomicBool::new(false));

    let handler = move || {
        if tripped.load(Ordering::Relaxed) {
            return true;
        }
        match deadline {
            Some(d) if Instant::now() >= d => {
                tripped.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    };

    conn.progress_handler(1000, Some(handler));
}

/// Prepare + execute the statement, collecting all rows into a typed result.
fn run_query(conn: &Connection, sql: &str) -> Result<QueryResult, TgError> {
    let mut stmt = conn.prepare(sql).map_err(map_prepare_error)?;

    let column_count = stmt.column_count();
    let columns: Vec<String> = (0..column_count)
        .map(|i| stmt.column_name(i).unwrap_or("").to_string())
        .collect();

    // Initialize column_types to Null; we'll refine as we see real values.
    let mut column_types: Vec<SqlType> = vec![SqlType::Null; column_count];

    let mut rows_out: Vec<Vec<SqlValue>> = Vec::new();
    let mut rows = stmt.query([]).map_err(map_execute_error)?;

    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut out_row = Vec::with_capacity(column_count);
                for (i, col_type) in column_types.iter_mut().enumerate().take(column_count) {
                    let value_ref = row.get_ref(i).map_err(map_execute_error)?;
                    let (sql_type, sql_value) = convert_value(value_ref);
                    // Update column_types: keep first non-Null observation.
                    if *col_type == SqlType::Null && sql_type != SqlType::Null {
                        *col_type = sql_type;
                    }
                    out_row.push(sql_value);
                }
                rows_out.push(out_row);
            }
            Ok(None) => break,
            Err(e) => return Err(map_execute_error(e)),
        }
    }

    Ok(QueryResult {
        columns,
        column_types,
        rows: rows_out,
    })
}

/// Convert a rusqlite `ValueRef` into our owned `(SqlType, SqlValue)` pair.
fn convert_value(v: ValueRef<'_>) -> (SqlType, SqlValue) {
    match v.data_type() {
        Type::Null => (SqlType::Null, SqlValue::Null),
        Type::Integer => {
            let i = v.as_i64().unwrap_or(0);
            (SqlType::Integer, SqlValue::Integer(i))
        }
        Type::Real => {
            let f = v.as_f64().unwrap_or(0.0);
            (SqlType::Real, SqlValue::Real(f))
        }
        Type::Text => {
            let s = v.as_str().unwrap_or("").to_string();
            (SqlType::Text, SqlValue::Text(s))
        }
        Type::Blob => {
            let b = v.as_blob().unwrap_or(&[]).to_vec();
            (SqlType::Blob, SqlValue::Blob(b))
        }
    }
}

/// Map errors that occur during `prepare`. Authorizer denials fire here (the
/// prepare step walks the AST and triggers the authorizer on each action).
fn map_prepare_error(e: rusqlite::Error) -> TgError {
    use rusqlite::ffi::ErrorCode;
    let msg = e.to_string();
    if let Some(sqlite_err) = e.sqlite_error() {
        match sqlite_err.code {
            ErrorCode::AuthorizationForStatementDenied => {
                return TgError::QueryDenied {
                    action: "statement contains a disallowed action".to_string(),
                    hint: "This sandbox allows SELECT only (plus PRAGMA table_info/index_list/index_info). See `tg query --schema`.".to_string(),
                };
            }
            ErrorCode::OperationInterrupted => {
                return TgError::QueryTimeout { limit_secs: 0 };
            }
            _ => {}
        }
    }
    // SQLite emits SQLITE_ERROR with "not authorized" for certain hard-coded
    // auth checks (e.g. `load_extension` when the extension API is disabled
    // at compile time). Surface these as QueryDenied so the user sees a
    // consistent sandbox-denial message.
    if msg.contains("not authorized") {
        return TgError::QueryDenied {
            action: "statement uses a disallowed function or action".to_string(),
            hint: "This sandbox blocks file-access and extension-loading functions.".to_string(),
        };
    }
    TgError::QuerySyntax { message: msg }
}

/// Map errors from `execute` / `row.next` / `row.get_ref`. Progress-handler
/// interrupts surface here, as can authorizer denials if a deferred action
/// fires during row iteration.
fn map_execute_error(e: rusqlite::Error) -> TgError {
    use rusqlite::ffi::ErrorCode;
    if let Some(sqlite_err) = e.sqlite_error() {
        match sqlite_err.code {
            ErrorCode::OperationInterrupted => {
                return TgError::QueryTimeout { limit_secs: 0 };
            }
            ErrorCode::AuthorizationForStatementDenied => {
                return TgError::QueryDenied {
                    action: "statement contains a disallowed action".to_string(),
                    hint: "This sandbox allows SELECT only.".to_string(),
                };
            }
            _ => {}
        }
    }
    TgError::QuerySyntax {
        message: e.to_string(),
    }
}
