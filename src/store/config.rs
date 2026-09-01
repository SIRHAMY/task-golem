use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::errors::TgError;

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {}

impl Config {
    /// Load config from `.task-golem/config.yaml`. Returns default config if file doesn't exist.
    pub fn load(project_dir: &Path) -> Result<Config, TgError> {
        let config_path = project_dir.join("config.yaml");
        if !config_path.exists() {
            return Ok(Config::default());
        }

        let content = fs::read_to_string(&config_path).map_err(TgError::IoError)?;
        if content.trim().is_empty() {
            return Ok(Config::default());
        }
        let config: Config = serde_yaml::from_str(&content)
            .map_err(|e| TgError::InvalidInput(format!("Invalid config.yaml: {}", e)))?;

        Ok(config)
    }
}
