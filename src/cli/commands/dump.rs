use std::io::{self, Write};

use task_golem::errors::TgError;
use task_golem::store::Store;
use task_golem::store::root;

/// Handle `tg dump [--yaml]`
///
/// Export all items (active + archive) in JSON or YAML format.
pub fn run(yaml: bool) -> Result<(), TgError> {
    let project_dir = root::find_project_root_from_cwd()?;
    let store = Store::new(project_dir);

    let active_items = store.load_active()?;
    let archive_items = store.load_all_archive()?;

    let output = serde_json::json!({
        "active": active_items,
        "archive": archive_items,
    });

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    if yaml {
        let yaml_str = serde_yaml::to_string(&output)
            .map_err(|e| TgError::InvalidInput(format!("Failed to serialize as YAML: {}", e)))?;
        write!(handle, "{}", yaml_str).map_err(TgError::IoError)?;
    } else {
        let json_str = serde_json::to_string_pretty(&output)
            .map_err(|e| TgError::InvalidInput(format!("Failed to serialize as JSON: {}", e)))?;
        writeln!(handle, "{}", json_str).map_err(TgError::IoError)?;
    }

    Ok(())
}
