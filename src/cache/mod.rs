//! SQLite cache over the authoritative JSONL store.
//!
//! The cache is pure derived state. It is rebuilt lazily from `tasks.jsonl` whenever
//! the composite stamp `(mtime_nanos, size, xxh3_64)` disagrees with what's recorded
//! in `_cache_meta`. The JSONL remains the single source of truth — if the cache is
//! deleted, the next `open_or_rebuild` call simply rebuilds it.
//!
//! ## Layout
//!
//! - `mod.rs` (this file) — public API, stamp logic, DDL constant, meta helpers.
//! - `rebuild.rs` — rebuild orchestration: lock → read → validate → temp DB → rename.
//! - `query.rs` (Phase 4) — read-only sandboxed query execution.
//!
//! Keep this module self-contained: nothing outside `src/cache/` should depend on
//! `rusqlite`.

pub mod rebuild;

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use xxhash_rust::xxh3::xxh3_64;

use crate::errors::TgError;
use crate::store::Store;

/// Cache schema version. Bump whenever `DDL` changes in a way that invalidates
/// existing `cache.db` files; `open_or_rebuild` compares this value to the one
/// stored in `_cache_meta` and forces a rebuild on mismatch.
pub const SCHEMA_VERSION: u32 = 1;

/// Full DDL for a fresh cache. Applied in one `execute_batch` call inside the
/// rebuild transaction. Exported so Phase 4's `tg query --schema` can render it.
///
/// Keep this string in sync with DESIGN §Cache Schema — Phase 4 renders this
/// verbatim (plus a columns summary) in the `--schema` output. When modifying:
/// 1. Update the matching reference section in the DESIGN doc.
/// 2. Bump `SCHEMA_VERSION`.
pub const DDL: &str = "\
CREATE TABLE _cache_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  priority INTEGER NOT NULL,
  description TEXT,
  parent TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  blocked_reason TEXT,
  blocked_from_status TEXT,
  claimed_by TEXT,
  claimed_at TEXT
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent);
CREATE INDEX idx_tasks_priority ON tasks(priority);

CREATE TABLE task_tags (
  task_id TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (task_id, tag)
);
CREATE INDEX idx_tags_tag ON task_tags(tag);

CREATE TABLE task_deps (
  task_id TEXT NOT NULL,
  dep_id TEXT NOT NULL,
  PRIMARY KEY (task_id, dep_id)
);
CREATE INDEX idx_deps_dep_id ON task_deps(dep_id);

CREATE TABLE task_view (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  priority INTEGER NOT NULL,
  parent TEXT,
  depth_from_root INTEGER NOT NULL,
  is_ready INTEGER NOT NULL,
  unmet_dep_count INTEGER NOT NULL
);
CREATE INDEX idx_view_status ON task_view(status);
CREATE INDEX idx_view_parent ON task_view(parent);
CREATE INDEX idx_view_ready ON task_view(is_ready);
";

/// Composite freshness stamp for a JSONL file.
///
/// All three fields must match for the cache to be considered fresh. `mtime` +
/// `size` is a cheap O(1) prefilter; `xxh3_64` is a robust content check that
/// catches edits that preserve size+mtime (unusual but possible with tooling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub mtime_nanos: i128,
    pub size: u64,
    pub xxh3_64: u64,
}

impl Stamp {
    /// Zero stamp for a missing-or-unreadable cache (forces rebuild).
    pub const ZERO: Stamp = Stamp {
        mtime_nanos: 0,
        size: 0,
        xxh3_64: 0,
    };
}

/// Compute the stamp for `jsonl_path`. Reads the whole file for hashing; for the
/// file sizes we target (≤ a few MB for 5k tasks) this is sub-millisecond.
///
/// Returns a `ZERO` stamp if the file is missing. Propagates other IO errors so
/// callers can distinguish "fresh project" from "unreadable filesystem".
pub fn compute_stamp(jsonl_path: &Path) -> Result<Stamp, TgError> {
    let metadata = match fs::metadata(jsonl_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stamp::ZERO),
        Err(e) => return Err(TgError::IoError(e)),
    };

    let mtime_nanos = metadata
        .modified()
        .map_err(TgError::IoError)?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);

    let bytes = fs::read(jsonl_path).map_err(TgError::IoError)?;
    Ok(Stamp {
        mtime_nanos,
        size: metadata.len(),
        xxh3_64: xxh3_64(&bytes),
    })
}

/// Read the stored stamp + schema_version from `_cache_meta`. Returns `None` if
/// any expected row is missing or malformed — callers treat that as "rebuild".
pub(crate) fn read_meta(conn: &Connection) -> Option<(u32, Stamp)> {
    let get = |key: &str| -> Option<String> {
        conn.query_row(
            "SELECT value FROM _cache_meta WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .ok()
    };

    let schema_version: u32 = get("schema_version")?.parse().ok()?;
    let mtime_nanos: i128 = get("jsonl_mtime_nanos")?.parse().ok()?;
    let size: u64 = get("jsonl_size")?.parse().ok()?;
    let xxh3_64: u64 = get("jsonl_xxh3")?.parse().ok()?;

    Some((
        schema_version,
        Stamp {
            mtime_nanos,
            size,
            xxh3_64,
        },
    ))
}

/// Write stamp + schema_version into `_cache_meta` inside the rebuild transaction.
pub(crate) fn write_meta(conn: &Connection, stamp: &Stamp) -> rusqlite::Result<()> {
    let mut stmt =
        conn.prepare("INSERT OR REPLACE INTO _cache_meta(key, value) VALUES (?1, ?2)")?;
    stmt.execute(["schema_version", &SCHEMA_VERSION.to_string()])?;
    stmt.execute(["jsonl_mtime_nanos", &stamp.mtime_nanos.to_string()])?;
    stmt.execute(["jsonl_size", &stamp.size.to_string()])?;
    stmt.execute(["jsonl_xxh3", &stamp.xxh3_64.to_string()])?;
    Ok(())
}

/// Open `cache.db` for read, rebuilding if stale or missing.
///
/// Returns an open read-only connection ready for query execution. If the cache
/// directory is unwritable (e.g. read-only filesystem), falls back to an in-memory
/// SQLite instance populated with a fresh rebuild — under `--verbose` the caller
/// sees a notice, but the query still runs.
///
/// ## Freshness protocol
///
/// 1. Cache missing → rebuild.
/// 2. Cache present but meta unreadable → treat as corrupt, rebuild.
/// 3. Schema version mismatch → rebuild.
/// 4. Stamp mismatch (mtime/size cheap check, xxh3 confirmation) → rebuild.
/// 5. All three match → open read-only, no rebuild.
pub fn open_or_rebuild(store: &Store, verbose: bool) -> Result<Connection, TgError> {
    let jsonl_path = store.tasks_jsonl_path();
    let cache_path = store.cache_db_path();

    let current_stamp = compute_stamp(&jsonl_path)?;

    let needs_rebuild = if !cache_path.exists() {
        true
    } else {
        // Open read-write briefly to read meta; re-open read-only for the caller.
        match Connection::open(&cache_path) {
            Ok(conn) => match read_meta(&conn) {
                Some((v, stamp)) => v != SCHEMA_VERSION || stamp != current_stamp,
                None => true,
            },
            Err(_) => true,
        }
    };

    if needs_rebuild {
        // Pre-check: if the project dir is unwritable, don't even try to build a
        // temp file — SQLite's "unable to open" error message doesn't expose a
        // clean permission-denied signal to match on, so we detect it up front.
        if !can_write_to(store.project_dir()) {
            if verbose {
                eprintln!(
                    "note: cache directory {} unwritable; using in-memory cache for this invocation",
                    store.project_dir().display()
                );
            }
            return rebuild::rebuild_in_memory(store);
        }

        rebuild::rebuild_to(store, &cache_path)?;
    }

    Connection::open_with_flags(&cache_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
        |e| TgError::CacheCorrupt {
            detail: format!("failed to open {}: {}", cache_path.display(), e),
        },
    )
}

/// Probe whether we can create a file in `dir`. Used to decide whether to fall
/// back to in-memory SQLite on read-only filesystems.
fn can_write_to(dir: &Path) -> bool {
    let probe = dir.join(format!(".cache-write-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
