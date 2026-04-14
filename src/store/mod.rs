pub mod config;
pub mod jsonl;
pub mod lock;
pub mod root;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::errors::TgError;
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
