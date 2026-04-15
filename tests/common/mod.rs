use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use task_golem::events::Event;

pub struct TestProject {
    tmp_dir: tempfile::TempDir,
}

impl TestProject {
    /// Create a new test project in a temp directory with `tg init` already run.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let tmp_dir = tempfile::tempdir()?;
        let project = TestProject { tmp_dir };

        // Run tg init
        let output = project.tg_cmd().arg("init").output()?;
        if !output.status.success() {
            return Err(format!(
                "tg init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(project)
    }

    /// Create a new test project without running init.
    pub fn new_uninit() -> Result<Self, Box<dyn std::error::Error>> {
        let tmp_dir = tempfile::tempdir()?;
        Ok(TestProject { tmp_dir })
    }

    /// Get the path to the test project directory.
    pub fn path(&self) -> &Path {
        self.tmp_dir.path()
    }

    /// Get the .task-golem directory path.
    pub fn project_dir(&self) -> PathBuf {
        self.tmp_dir.path().join(".task-golem")
    }

    /// Create a Command for running `tg` in this project's directory.
    pub fn tg_cmd(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("tg"));
        cmd.current_dir(self.tmp_dir.path());
        cmd
    }

    /// Run a tg command and return the output.
    pub fn run_tg(&self, args: &[&str]) -> std::process::Output {
        self.tg_cmd()
            .args(args)
            .output()
            .expect("failed to execute tg")
    }

    /// Run a tg command and parse the JSON output.
    pub fn run_tg_json(&self, args: &[&str]) -> serde_json::Value {
        let mut all_args = vec!["--json"];
        all_args.extend_from_slice(args);
        let output = self.run_tg(&all_args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout))
    }

    /// Write raw events directly into `events.jsonl` and `events.archive.jsonl`.
    ///
    /// Useful for doctor tests that need to produce states (drift, dup,
    /// orphan) that can't be driven through the CLI without racing the
    /// store. Overwrites any existing content in both files. Passes through
    /// whatever `Event` the caller constructs — callers are responsible for
    /// timestamps, task_ids, and types.
    #[allow(dead_code)]
    pub fn seed_raw_events(&self, active: &[Event], archive: &[Event]) {
        let project_dir = self.project_dir();
        let active_path = project_dir.join("events.jsonl");
        let archive_path = project_dir.join("events.archive.jsonl");
        write_events_file(&active_path, active);
        write_events_file(&archive_path, archive);
    }
}

fn write_events_file(path: &Path, events: &[Event]) {
    // Create dir just in case (project_dir() should exist after `tg init`,
    // but tests using `new_uninit` may invoke this helper too).
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok();
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .expect("open events file");
    for event in events {
        let line = serde_json::to_string(event).expect("serialize event");
        writeln!(f, "{}", line).expect("write event line");
    }
}
