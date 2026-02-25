use std::path::{Path, PathBuf};

use crate::errors::TgError;

const PROJECT_DIR: &str = ".task-golem";

/// Walk parent directories from `start` looking for `.task-golem/`.
/// Returns the path to the `.task-golem/` directory, or NotInitialized error.
pub fn find_project_root(start: &Path) -> Result<PathBuf, TgError> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(PROJECT_DIR);
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !current.pop() {
            return Err(TgError::NotInitialized(start.display().to_string()));
        }
    }
}

/// Return the `.task-golem/` directory path from CWD.
pub fn find_project_root_from_cwd() -> Result<PathBuf, TgError> {
    let cwd = std::env::current_dir().map_err(TgError::IoError)?;
    find_project_root(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_in_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(PROJECT_DIR)).unwrap();
        let result = find_project_root(tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.path().join(PROJECT_DIR));
    }

    #[test]
    fn finds_in_parent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(PROJECT_DIR)).unwrap();
        let child = tmp.path().join("subdir");
        std::fs::create_dir(&child).unwrap();
        let result = find_project_root(&child);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.path().join(PROJECT_DIR));
    }

    #[test]
    fn finds_in_grandparent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(PROJECT_DIR)).unwrap();
        let child = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&child).unwrap();
        let result = find_project_root(&child);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), tmp.path().join(PROJECT_DIR));
    }

    #[test]
    fn returns_error_when_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        // No .task-golem/ created
        let result = find_project_root(tmp.path());
        assert!(matches!(result, Err(TgError::NotInitialized(_))));
    }
}
