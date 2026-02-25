use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin;

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
        let mut cmd = Command::new(cargo_bin("tg"));
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
}
