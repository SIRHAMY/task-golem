use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::cli::output;
use crate::errors::TgError;
use crate::store::jsonl;

const PROJECT_DIR: &str = ".task-golem";

#[derive(Debug, Serialize)]
struct InitOutput {
    initialized: bool,
    path: String,
}

pub fn run(json_mode: bool, force: bool) -> Result<(), TgError> {
    let cwd = std::env::current_dir().map_err(TgError::IoError)?;
    let project_dir = cwd.join(PROJECT_DIR);

    if project_dir.exists() && !force {
        return Err(TgError::InvalidInput(format!(
            "Project already initialized at {}. Use --force to reinitialize.",
            project_dir.display()
        )));
    }

    if project_dir.exists() && force {
        eprintln!(
            "Warning: Reinitializing existing project at {}. Existing data will be overwritten.",
            project_dir.display()
        );
    }

    create_project(&project_dir)?;

    let result = InitOutput {
        initialized: true,
        path: format!("{}/", PROJECT_DIR),
    };

    output::output(
        json_mode,
        &result,
        &format!("Initialized task-golem project at {}/", PROJECT_DIR),
    );

    Ok(())
}

fn create_project(project_dir: &Path) -> Result<(), TgError> {
    fs::create_dir_all(project_dir).map_err(TgError::IoError)?;

    // Write empty tasks.jsonl with schema header
    jsonl::write_empty(&project_dir.join("tasks.jsonl"))?;

    // Write empty archive.jsonl with schema header
    jsonl::write_empty(&project_dir.join("archive.jsonl"))?;

    // Create empty lock file
    fs::File::create(project_dir.join("tasks.lock")).map_err(TgError::IoError)?;

    Ok(())
}
