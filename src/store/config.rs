use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::errors::TgError;

const DEFAULT_ID_PREFIX: &str = "tg";

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default = "default_id_prefix")]
    pub id_prefix: String,
}

fn default_id_prefix() -> String {
    DEFAULT_ID_PREFIX.to_string()
}

impl Config {
    /// Load config from `.task-golem/config.yaml`. Returns default config if file doesn't exist.
    pub fn load(project_dir: &Path) -> Result<Config, TgError> {
        let config_path = project_dir.join("config.yaml");
        if !config_path.exists() {
            return Ok(Config {
                id_prefix: DEFAULT_ID_PREFIX.to_string(),
            });
        }

        let content = fs::read_to_string(&config_path).map_err(TgError::IoError)?;
        let config: Config = serde_yaml::from_str(&content)
            .map_err(|e| TgError::InvalidInput(format!("Invalid config.yaml: {}", e)))?;

        Ok(config)
    }
}
