pub mod config;
pub mod jsonl;
pub mod lock;
pub mod root;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::errors::TgError;
use crate::events::record::Event;
use crate::events::witness::StatusChange;
use crate::events::{append as events_append, archive as events_archive, author as events_author};
use crate::model::item::Item;

/// Gitignore lines that keep the SQLite cache out of git.
/// Exposed for both `Store::ensure_gitignore` and doctor checks.
pub const CACHE_GITIGNORE_LINES: &[&str] = &["cache.db", "cache.db-journal", "cache.db.tmp-*"];

#[derive(Clone)]
pub struct Store {
    project_dir: PathBuf,
}

/// Cached path state derived from the project directory.
///
/// Kept as methods instead of fields so the store stays cheaply cloneable.
impl Store {
    /// Path to the cache database. Shared between `cache::open_or_rebuild` and doctor.
    pub fn cache_db_path(&self) -> PathBuf {
        self.project_dir.join("cache.db")
    }

    /// Path to the cache-module gitignore (lives alongside cache.db).
    pub fn gitignore_path(&self) -> PathBuf {
        self.project_dir.join(".gitignore")
    }
}

impl Store {
    pub fn new(project_dir: PathBuf) -> Self {
        Store { project_dir }
    }

    pub fn tasks_path(&self) -> PathBuf {
        self.project_dir.join("tasks.jsonl")
    }

    /// Borrowed accessor used by the cache module (avoids an extra PathBuf clone
    /// on every rebuild). Kept distinct from `tasks_path` to keep ownership
    /// semantics clear at the call site.
    pub fn tasks_jsonl_path(&self) -> PathBuf {
        self.tasks_path()
    }

    /// Project directory (`.task-golem/`). Exposed for the cache module.
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub fn archive_path(&self) -> PathBuf {
        self.project_dir.join("archive.jsonl")
    }

    /// Path to the active event log (`events.jsonl`).
    pub fn events_path(&self) -> PathBuf {
        self.project_dir.join("events.jsonl")
    }

    /// Path to the archived event log (`events.archive.jsonl`).
    ///
    /// Populated by `events::archive::move_for_task` in Phase 3; the
    /// accessor is introduced now so `Store::commit_done` and the Phase 3
    /// archive move share the same path contract.
    pub fn events_archive_path(&self) -> PathBuf {
        self.project_dir.join("events.archive.jsonl")
    }

    pub fn lock_path(&self) -> PathBuf {
        self.project_dir.join("tasks.lock")
    }

    /// Acquire the file lock, execute a callback, release on drop.
    pub fn with_lock<F, R>(&self, callback: F) -> Result<R, TgError>
    where
        F: FnOnce(&Self) -> Result<R, TgError>,
    {
        let lock_path = self.lock_path();
        lock::with_lock(&lock_path, || callback(self))
    }

    /// Load active items. Safe to call without a lock for read-only operations
    /// (atomic rename guarantees consistent snapshots). For read-modify-write,
    /// callers must use `with_lock` and load inside the callback.
    pub fn load_active(&self) -> Result<Vec<Item>, TgError> {
        jsonl::read_active(&self.tasks_path())
    }

    /// Save active items atomically (must be called under lock).
    pub fn save_active(&self, items: &[Item]) -> Result<(), TgError> {
        jsonl::write_atomic(&self.tasks_path(), items)
    }

    /// Scan archive line-by-line extracting only IDs (fast path).
    pub fn load_archive_ids(&self) -> Result<HashSet<String>, TgError> {
        let items = jsonl::read_archive(&self.archive_path())?;
        Ok(items.into_iter().map(|item| item.id).collect())
    }

    /// Scan archive for a specific item.
    pub fn load_archive_item(&self, id: &str) -> Result<Option<Item>, TgError> {
        let items = jsonl::read_archive(&self.archive_path())?;
        Ok(items.into_iter().find(|item| item.id == id))
    }

    /// Full archive deserialization.
    pub fn load_all_archive(&self) -> Result<Vec<Item>, TgError> {
        jsonl::read_archive(&self.archive_path())
    }

    /// Union of active IDs + archive IDs.
    pub fn all_known_ids(&self) -> Result<HashSet<String>, TgError> {
        let active = self.load_active()?;
        let archive_ids = self.load_archive_ids()?;
        let mut all = archive_ids;
        for item in active {
            all.insert(item.id);
        }
        Ok(all)
    }

    /// Append a single item to the archive file.
    pub fn append_to_archive(&self, item: &Item) -> Result<(), TgError> {
        jsonl::append_to_archive(&self.archive_path(), item)
    }

    /// Redeem a [`StatusChange`] witness for a non-terminal transition.
    ///
    /// Ordering: append the event to `events.jsonl` (fsynced) first, **then**
    /// rewrite `tasks.jsonl`. A crash between the two steps leaves a
    /// committed event with no task mutation — surfaced by the doctor drift
    /// check in Phase 5. The reverse (task mutation without event) would be
    /// invisible and is therefore forbidden by construction (the witness is
    /// consumed here, so callers cannot skip the event).
    ///
    /// # Locking
    ///
    /// Must be called from inside a [`Store::with_lock`] closure. The lock
    /// contract is documented rather than type-enforced (enforcing it would
    /// require redesigning the lock API, which is out of scope).
    pub fn commit_status_change(
        &self,
        items: &[Item],
        change: StatusChange,
    ) -> Result<(), TgError> {
        let (task_id, new_status, text) = change.fields();
        let author = events_author::resolve();
        let event = Event::status_transition(task_id, author, new_status, text);
        events_append::write(&self.events_path(), &event)?;
        jsonl::write_atomic(&self.tasks_path(), items)?;
        Ok(())
    }

    /// Redeem a [`StatusChange`] witness for a `done` transition that archives
    /// the item.
    ///
    /// Ordering: append the `status_transition` event (fsynced), then append
    /// the done item to `archive.jsonl` (fsynced), then rewrite
    /// `tasks.jsonl` WITHOUT the done item. Caller must have already removed
    /// the done item from `items`.
    ///
    /// # Locking
    ///
    /// Must be called from inside a [`Store::with_lock`] closure.
    pub fn commit_done(
        &self,
        items: &[Item],
        done_item: &Item,
        change: StatusChange,
    ) -> Result<(), TgError> {
        let (task_id, new_status, text) = change.fields();
        let author = events_author::resolve();
        let event = Event::status_transition(task_id, author, new_status, text);
        events_append::write(&self.events_path(), &event)?;
        self.append_to_archive(done_item)?;
        jsonl::write_atomic(&self.tasks_path(), items)?;
        // Move every event for this task from events.jsonl to
        // events.archive.jsonl. Runs last so the task mutation is durable
        // before we touch the events files again; a crash after tasks.jsonl
        // rewrites but before the move completes leaves events stranded in
        // active — surfaced by the Phase 5 `events_in_active_for_archived_task`
        // doctor check.
        events_archive::move_for_task(&self.events_path(), &self.events_archive_path(), task_id)?;
        Ok(())
    }

    /// Append a free-text note event for `task_id`.
    ///
    /// Validates that `task_id` exists in the ACTIVE store (archived tasks
    /// are rejected; the CLI layer in Phase 4 surfaces the rejection with a
    /// user-facing hint). Validation is lock-free — it relies on `O_APPEND`
    /// atomicity for the write itself. A race where the task archives
    /// between validate and append is acceptable: the resulting late note
    /// becomes an `events_in_active_for_archived_task` condition caught by
    /// the Phase 5 doctor check.
    ///
    /// Returns the `Event` that was appended (useful for CLI output).
    pub fn append_note(&self, task_id: &str, text: &str) -> Result<Event, TgError> {
        let items = self.load_active()?;
        if !items.iter().any(|i| i.id == task_id) {
            return Err(TgError::ItemNotFound(task_id.to_string()));
        }
        let author = events_author::resolve();
        let event = Event::note(task_id, author, text);
        events_append::write(&self.events_path(), &event)?;
        Ok(event)
    }

    /// Ensure `.task-golem/.gitignore` exists and covers all cache-related artifacts.
    ///
    /// Idempotent: reads existing lines, appends only missing ones. Preserves the
    /// existing file's trailing-newline shape so user-managed lines are untouched.
    pub fn ensure_gitignore(&self) -> Result<(), TgError> {
        use std::fs;
        use std::io::Write;

        let path = self.gitignore_path();
        let existing = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(TgError::IoError(e)),
        };

        let present: HashSet<&str> = existing
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        let missing: Vec<&str> = CACHE_GITIGNORE_LINES
            .iter()
            .copied()
            .filter(|line| !present.contains(line))
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        // Append with a leading newline if the existing file doesn't already end in one
        // (preserves user's existing file shape).
        let needs_leading_newline = !existing.is_empty() && !existing.ends_with('\n');

        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(TgError::IoError)?;

        if needs_leading_newline {
            writeln!(f).map_err(TgError::IoError)?;
        }

        for line in missing {
            writeln!(f, "{}", line).map_err(TgError::IoError)?;
        }

        f.sync_all().map_err(TgError::IoError)?;
        Ok(())
    }
}
