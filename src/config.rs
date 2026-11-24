//! The app's own settings (currently just the optional SteamGridDB API key).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// SteamGridDB API key, for downloading artwork for new shortcuts. Empty = no artwork.
    pub steamgriddb_key: String,
}

fn path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join(crate::EXE_STEM)
            .join("config.json"),
    )
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = path().ok_or_else(|| anyhow::anyhow!("no settings folder"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
