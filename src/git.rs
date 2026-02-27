use std::path::Path;
use std::process::Command;

use crate::errors::TgError;

const TASKS_FILE: &str = ".task-golem/tasks.jsonl";
const ARCHIVE_FILE: &str = ".task-golem/archive.jsonl";

/// Stage task-golem's own data files (tasks.jsonl and archive.jsonl).
///
/// Only stages these two files -- does not stage arbitrary paths.
/// `project_dir` is the repository root (parent of `.task-golem/`).
pub fn stage_self(project_dir: &Path) -> Result<(), TgError> {
    run_git_command(
        &["add", "--", TASKS_FILE, ARCHIVE_FILE],
        project_dir,
    )
    .map(|_| ())
}

/// Commit all currently-staged changes and return the new commit SHA.
///
/// `project_dir` is the repository root. The caller is responsible for
/// staging the desired files before calling this function.
pub fn commit(message: &str, project_dir: &Path) -> Result<String, TgError> {
    run_git_command(&["commit", "-m", message], project_dir)?;
    let sha = run_git_command(&["rev-parse", "HEAD"], project_dir)?;
    Ok(sha.trim().to_string())
}

fn run_git_command(args: &[&str], repo_dir: &Path) -> Result<String, TgError> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    cmd.current_dir(repo_dir);

    let output = cmd.output().map_err(|e| {
        TgError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to run git {}: {}", args.first().unwrap_or(&""), e),
        ))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TgError::IoError(std::io::Error::other(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        ))));
    }

    String::from_utf8(output.stdout).map_err(|e| {
        TgError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("git output is not valid UTF-8: {}", e),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Set up a temporary directory with a git repo and `.task-golem/` data files.
    fn setup_git_repo(dir: &Path) {
        run_test_git(&["init"], dir);
        run_test_git(&["config", "user.email", "test@test.com"], dir);
        run_test_git(&["config", "user.name", "Test"], dir);

        // Create .task-golem directory with data files
        let tg_dir = dir.join(".task-golem");
        fs::create_dir_all(&tg_dir).unwrap();
        fs::write(tg_dir.join("tasks.jsonl"), "{\"schema_version\":1}\n").unwrap();
        fs::write(tg_dir.join("archive.jsonl"), "{\"schema_version\":1}\n").unwrap();

        // Initial commit so HEAD exists
        run_test_git(&["add", "."], dir);
        run_test_git(&["commit", "-m", "Initial commit"], dir);
    }

    fn run_test_git(args: &[&str], dir: &Path) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command failed to execute");
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn stage_self_stages_task_golem_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        setup_git_repo(dir);

        // Modify tasks.jsonl
        let tasks_path = dir.join(".task-golem/tasks.jsonl");
        fs::write(&tasks_path, "{\"schema_version\":1}\n{}\n").unwrap();

        stage_self(dir).unwrap();

        // Check that the file is staged
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(dir)
            .output()
            .unwrap();
        let status_str = String::from_utf8_lossy(&status.stdout);
        assert!(
            status_str.contains(".task-golem/tasks.jsonl"),
            "tasks.jsonl should be staged: {}",
            status_str
        );
    }

    #[test]
    fn commit_creates_commit_and_returns_sha() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        setup_git_repo(dir);

        // Modify and stage a file
        let tasks_path = dir.join(".task-golem/tasks.jsonl");
        fs::write(&tasks_path, "{\"schema_version\":1}\n{}\n").unwrap();
        stage_self(dir).unwrap();

        let sha = commit("Test commit message", dir).unwrap();

        // SHA should be 40 hex characters
        assert_eq!(sha.len(), 40, "SHA should be 40 chars: {}", sha);
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex: {}",
            sha
        );

        // Verify the commit message
        let log = Command::new("git")
            .args(["log", "-1", "--format=%s"])
            .current_dir(dir)
            .output()
            .unwrap();
        let message = String::from_utf8_lossy(&log.stdout).trim().to_string();
        assert_eq!(message, "Test commit message");
    }

    #[test]
    fn commit_fails_with_nothing_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        setup_git_repo(dir);

        // No changes staged -- commit should fail
        let result = commit("Empty commit", dir);
        assert!(result.is_err(), "Commit with nothing staged should fail");
    }

    #[test]
    fn stage_self_with_no_changes_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        setup_git_repo(dir);

        // No modifications -- stage_self should succeed (git add of unchanged files is a no-op)
        let result = stage_self(dir);
        assert!(result.is_ok(), "Staging unchanged files should succeed");
    }

    #[test]
    fn commit_includes_other_staged_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        setup_git_repo(dir);

        // Stage an arbitrary file before calling stage_self + commit
        let extra_file = dir.join("extra.txt");
        fs::write(&extra_file, "extra content").unwrap();
        run_test_git(&["add", "extra.txt"], dir);

        // Also modify task-golem files
        let tasks_path = dir.join(".task-golem/tasks.jsonl");
        fs::write(&tasks_path, "{\"schema_version\":1}\n{}\n").unwrap();
        stage_self(dir).unwrap();

        let sha = commit("Combined commit", dir).unwrap();
        assert!(!sha.is_empty());

        // Verify both files are in the commit
        let show = Command::new("git")
            .args(["diff-tree", "--no-commit-id", "--name-only", "-r", &sha])
            .current_dir(dir)
            .output()
            .unwrap();
        let files = String::from_utf8_lossy(&show.stdout);
        assert!(
            files.contains(".task-golem/tasks.jsonl"),
            "Commit should include tasks.jsonl: {}",
            files
        );
        assert!(
            files.contains("extra.txt"),
            "Commit should include extra.txt: {}",
            files
        );
    }
}
