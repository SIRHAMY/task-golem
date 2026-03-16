use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::errors::TgError;

const DEFAULT_ID_PREFIX: &str = "tg";
const DEFAULT_ID_HEX_LEN: usize = 5;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default = "default_id_prefix")]
    pub id_prefix: String,
    #[serde(default = "default_id_hex_len")]
    pub id_hex_len: usize,
}

fn default_id_prefix() -> String {
    DEFAULT_ID_PREFIX.to_string()
}

fn default_id_hex_len() -> usize {
    DEFAULT_ID_HEX_LEN
}

impl Config {
    /// Load config from `.task-golem/config.yaml`. Returns default config if file doesn't exist.
    pub fn load(project_dir: &Path) -> Result<Config, TgError> {
        let config_path = project_dir.join("config.yaml");
        if !config_path.exists() {
            return Ok(Config {
                id_prefix: DEFAULT_ID_PREFIX.to_string(),
                id_hex_len: DEFAULT_ID_HEX_LEN,
            });
        }

        let content = fs::read_to_string(&config_path).map_err(TgError::IoError)?;
        let config: Config = serde_yaml::from_str(&content)
            .map_err(|e| TgError::InvalidInput(format!("Invalid config.yaml: {}", e)))?;

        if config.id_hex_len < 3 || config.id_hex_len > 12 {
            return Err(TgError::InvalidInput(
                "id_hex_len must be between 3 and 12".to_string(),
            ));
        }

        Ok(config)
    }
}
