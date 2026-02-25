pub mod jsonl;
pub mod lock;
pub mod root;

use std::collections::HashSet;
use std::path::PathBuf;

use crate::errors::TgError;
use crate::model::item::Item;

pub struct Store {
    project_dir: PathBuf,
}

impl Store {
    pub fn new(project_dir: PathBuf) -> Self {
        Store { project_dir }
    }

    pub fn tasks_path(&self) -> PathBuf {
        self.project_dir.join("tasks.jsonl")
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
    #[allow(dead_code)] // Used in later phases
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
}
