//! Rebuild the SQLite cache from the JSONL source of truth.
//!
//! Flow (mirrors DESIGN §"Cache rebuild (internal)"):
//! 1. Acquire `store.with_lock()` for the JSONL read phase only.
//! 2. Strict-parse active items, validate no duplicates, detect cycles (dep + parent).
//! 3. Compute the stamp _before_ releasing the lock (prevents TOCTOU drift).
//! 4. Release the lock.
//! 5. Sweep stale `cache.db.tmp-*` from crashed prior runs.
//! 6. Build a temp DB (`cache.db.tmp-<pid>`) with MEMORY journal + sync=OFF.
//! 7. Apply DDL, BEGIN IMMEDIATE, bulk-insert, populate `task_view`, write meta.
//! 8. COMMIT, close, fsync, atomic rename over `cache.db`.
//! 9. Best-effort `ensure_gitignore` on first rebuild.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use rusqlite::{Connection, params};

use super::{DDL, Stamp, compute_stamp, write_meta};
use crate::errors::TgError;
use crate::model::deps;
use crate::model::item::Item;
use crate::model::status::Status;
use crate::store::Store;

/// Rebuild the cache from the JSONL at `store.tasks_jsonl_path()` and atomically
/// install it at `cache_path`.
pub fn rebuild_to(store: &Store, cache_path: &Path) -> Result<(), TgError> {
    // Phase 1: lock-and-read. Keep the lock scope as tight as possible so concurrent
    // writes aren't starved by the cache rebuild.
    let (items, stamp) = store.with_lock(|s| {
        let items = s.load_active()?;
        validate_for_rebuild(&items)?;
        let stamp = compute_stamp(&s.tasks_jsonl_path())?;
        Ok((items, stamp))
    })?;

    // Phase 2: build the temp DB outside the lock.
    let project_dir = store.project_dir();
    sweep_stale_tmps(project_dir);

    let tmp_path = project_dir.join(format!("cache.db.tmp-{}", process::id()));
    // If a tmp file from a previous crash with the same PID exists, remove it.
    let _ = fs::remove_file(&tmp_path);

    // Build; on any failure clean up the tmp file before returning.
    let build_result = build_temp_db(&tmp_path, &items, &stamp);
    if build_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    build_result?;

    // Phase 3: fsync the temp file, then atomic rename.
    fsync_file(&tmp_path)?;
    fs::rename(&tmp_path, cache_path).map_err(TgError::IoError)?;

    // Best-effort: ensure the cache artifacts are in .gitignore. Failures here
    // are non-fatal — the cache is built, the user can add lines manually.
    let _ = store.ensure_gitignore();

    Ok(())
}

/// Rebuild into an in-memory SQLite DB (used when `cache.db` path is unwritable).
pub fn rebuild_in_memory(store: &Store) -> Result<Connection, TgError> {
    let (items, stamp) = store.with_lock(|s| {
        let items = s.load_active()?;
        validate_for_rebuild(&items)?;
        let stamp = compute_stamp(&s.tasks_jsonl_path())?;
        Ok((items, stamp))
    })?;

    let conn = Connection::open_in_memory().map_err(|e| TgError::CacheRebuildFailed {
        detail: e.to_string(),
    })?;
    populate_connection(&conn, &items, &stamp)?;
    Ok(conn)
}

/// Check invariants the cache relies on: no duplicate IDs, no cycles on either
/// graph. Cycles must be rejected before we touch SQL — the `task_view` CTE
/// carries `WHERE depth < 64` as belt-and-suspenders, but the Rust-side check
/// produces a much clearer error message.
fn validate_for_rebuild(items: &[Item]) -> Result<(), TgError> {
    // Duplicate IDs.
    let mut seen: HashSet<&str> = HashSet::with_capacity(items.len());
    for item in items {
        if !seen.insert(&item.id) {
            return Err(TgError::StorageCorruption(format!(
                "duplicate task id in tasks.jsonl: {}",
                item.id
            )));
        }
    }

    // Dependency cycles.
    let dep_cycles = deps::detect_all_cycles(items);
    if !dep_cycles.is_empty() {
        let summary = dep_cycles
            .iter()
            .map(|c| c.join(" -> "))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(TgError::CycleDetected(format!(
            "dependency cycle(s) in tasks.jsonl: {} (run `tg doctor --fix` to repair)",
            summary
        )));
    }

    // Parent cycles.
    let parent_cycles = deps::detect_all_parent_cycles(items);
    if !parent_cycles.is_empty() {
        let ids: Vec<String> = parent_cycles.into_iter().flatten().collect();
        return Err(TgError::ParentCycle { ids });
    }

    Ok(())
}

/// Best-effort cleanup of leftover temp files from crashed rebuilds.
///
/// Only unlinks `cache.db.tmp-<pid>` files whose PID is not currently running —
/// otherwise a concurrent rebuild in a peer process would lose its in-flight
/// temp file. Files with unparseable suffixes are left alone (not our problem).
fn sweep_stale_tmps(project_dir: &Path) {
    let entries = match fs::read_dir(project_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        let Some(pid_str) = name.strip_prefix("cache.db.tmp-") else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        if !pid_is_running(pid) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Check whether a PID is currently running. Best-effort; on platforms where we
/// can't tell, returns `true` to err on the side of leaving files alone.
#[cfg(unix)]
fn pid_is_running(pid: u32) -> bool {
    // `kill(pid, 0)` returns 0 if the process exists (and we can signal it) or
    // EPERM if it exists but we can't. ESRCH means no such process.
    // SAFETY: `kill(pid, 0)` with sig=0 performs error checking without
    // actually sending a signal — it has no side effects.
    let result = unsafe { libc_kill(pid as i32, 0) };
    if result == 0 {
        true
    } else {
        // errno == ESRCH (3) means the process doesn't exist.
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        errno != 3 // ESRCH
    }
}

#[cfg(not(unix))]
fn pid_is_running(_pid: u32) -> bool {
    // Conservative on non-Unix: don't sweep.
    true
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// Build the full cache into a new file at `tmp_path`. Leaves `tmp_path` closed
/// and fsync-ready on success; caller handles fsync + rename.
fn build_temp_db(tmp_path: &PathBuf, items: &[Item], stamp: &Stamp) -> Result<(), TgError> {
    let conn = Connection::open(tmp_path).map_err(|e| TgError::CacheRebuildFailed {
        detail: format!("opening temp DB {}: {}", tmp_path.display(), e),
    })?;

    populate_connection(&conn, items, stamp)?;

    // Explicitly close so the OS flushes file metadata before we rename.
    conn.close().map_err(|(_, e)| TgError::CacheRebuildFailed {
        detail: format!("closing temp DB: {}", e),
    })?;

    Ok(())
}

/// Populate an open SQLite connection (file-backed or in-memory) with the full
/// cache: schema + rows + materialized view + meta.
fn populate_connection(conn: &Connection, items: &[Item], stamp: &Stamp) -> Result<(), TgError> {
    // Temp file or in-memory: durability pragmas don't apply (a crash discards
    // the temp; in-memory is ephemeral by definition).
    conn.execute_batch(
        "PRAGMA journal_mode = MEMORY;\n\
         PRAGMA synchronous = OFF;\n\
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| TgError::CacheRebuildFailed {
        detail: format!("pragmas: {}", e),
    })?;

    conn.execute_batch(DDL)
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("applying DDL: {}", e),
        })?;

    // All inserts happen in one transaction. BEGIN IMMEDIATE grabs the write lock
    // up front so we fail fast if something else has it (shouldn't happen — temp
    // file is private to this process).
    conn.execute_batch("BEGIN IMMEDIATE;")
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("begin: {}", e),
        })?;

    // Scope the prepared statements so they're dropped before COMMIT (rusqlite
    // holds a borrow on the connection while a Statement is alive).
    {
        insert_tasks(conn, items)?;
        insert_tags(conn, items)?;
        insert_deps(conn, items)?;
        populate_task_view(conn, items)?;
        write_meta(conn, stamp).map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("writing meta: {}", e),
        })?;
    }

    conn.execute_batch("COMMIT;")
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("commit: {}", e),
        })?;

    Ok(())
}

fn insert_tasks(conn: &Connection, items: &[Item]) -> Result<(), TgError> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO tasks(
                id, title, status, priority, description, parent,
                created_at, updated_at, blocked_reason, blocked_from_status,
                claimed_by, claimed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("prepare tasks: {}", e),
        })?;

    for item in items {
        stmt.execute(params![
            item.id,
            item.title,
            status_str(item.status),
            item.priority,
            item.description,
            item.parent,
            item.created_at.to_rfc3339(),
            item.updated_at.to_rfc3339(),
            item.blocked_reason,
            item.blocked_from_status.map(status_str),
            item.claimed_by,
            item.claimed_at.map(|dt| dt.to_rfc3339()),
        ])
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("insert task {}: {}", item.id, e),
        })?;
    }

    Ok(())
}

fn insert_tags(conn: &Connection, items: &[Item]) -> Result<(), TgError> {
    let mut stmt = conn
        .prepare("INSERT OR IGNORE INTO task_tags(task_id, tag) VALUES (?1, ?2)")
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("prepare tags: {}", e),
        })?;

    for item in items {
        for tag in &item.tags {
            stmt.execute(params![item.id, tag])
                .map_err(|e| TgError::CacheRebuildFailed {
                    detail: format!("insert tag for {}: {}", item.id, e),
                })?;
        }
    }

    Ok(())
}

fn insert_deps(conn: &Connection, items: &[Item]) -> Result<(), TgError> {
    let mut stmt = conn
        .prepare("INSERT OR IGNORE INTO task_deps(task_id, dep_id) VALUES (?1, ?2)")
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("prepare deps: {}", e),
        })?;

    for item in items {
        for dep in &item.dependencies {
            stmt.execute(params![item.id, dep])
                .map_err(|e| TgError::CacheRebuildFailed {
                    detail: format!("insert dep for {}: {}", item.id, e),
                })?;
        }
    }

    Ok(())
}

/// Materialize `task_view`.
///
/// `depth_from_root`, `is_ready`, and `unmet_dep_count` are all computed in Rust
/// before being written in one bulk insert. This is simpler than a recursive-CTE
/// UPDATE and faster at our scale (5k tasks rebuild in ~35ms end-to-end). The
/// depth computation clamps at 64 as defense-in-depth — cycles were already
/// ruled out by `validate_for_rebuild`, but the clamp matches the `depth < 64`
/// bound the SPEC requires for any recursive CTE over `tasks.parent`.
fn populate_task_view(conn: &Connection, items: &[Item]) -> Result<(), TgError> {
    // Build parent map for depth computation in Rust (avoids a second SQL pass;
    // cycles were already ruled out by validate_for_rebuild, so this terminates).
    let by_id: HashMap<&str, &Item> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    let mut depths: HashMap<&str, i64> = HashMap::with_capacity(items.len());

    fn depth_of<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a Item>,
        depths: &mut HashMap<&'a str, i64>,
    ) -> i64 {
        if let Some(&d) = depths.get(id) {
            return d;
        }
        let item = match by_id.get(id) {
            Some(i) => i,
            None => {
                // Dangling parent ref — treat as root for depth purposes. Doctor
                // will catch this separately.
                depths.insert(id, 0);
                return 0;
            }
        };
        let d = match &item.parent {
            Some(p) if by_id.contains_key(p.as_str()) => {
                // Defense-in-depth: clamp at 64 even though validate_for_rebuild
                // already proved acyclic.
                let parent_depth = depth_of(p.as_str(), by_id, depths);
                (parent_depth + 1).min(64)
            }
            _ => 0,
        };
        depths.insert(id, d);
        d
    }

    // Compute ready/unmet using the active set. A dep is "met" if the target is
    // present in the active set with status=done. Missing targets (dangling deps
    // or archived targets) count as unmet — matching the existing ready-queue
    // semantics.
    let done_ids: HashSet<&str> = items
        .iter()
        .filter(|i| i.status == Status::Done)
        .map(|i| i.id.as_str())
        .collect();

    let mut stmt = conn
        .prepare(
            "INSERT INTO task_view(
                id, title, status, priority, parent,
                depth_from_root, is_ready, unmet_dep_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("prepare task_view: {}", e),
        })?;

    for item in items {
        let depth = depth_of(&item.id, &by_id, &mut depths);
        let unmet = item
            .dependencies
            .iter()
            .filter(|d| !done_ids.contains(d.as_str()))
            .count() as i64;
        let is_ready = (item.status == Status::Todo && unmet == 0) as i64;

        stmt.execute(params![
            item.id,
            item.title,
            status_str(item.status),
            item.priority,
            item.parent,
            depth,
            is_ready,
            unmet,
        ])
        .map_err(|e| TgError::CacheRebuildFailed {
            detail: format!("insert task_view {}: {}", item.id, e),
        })?;
    }

    Ok(())
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Todo => "todo",
        Status::Doing => "doing",
        Status::Done => "done",
        Status::Blocked => "blocked",
    }
}

fn fsync_file(path: &Path) -> Result<(), TgError> {
    let f = fs::File::open(path).map_err(TgError::IoError)?;
    f.sync_all().map_err(TgError::IoError)?;
    Ok(())
}
