//! The app's own settings: the optional SteamGridDB API key, per-game launch timings, and
//! whether to check for new versions.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// SteamGridDB API key, for downloading artwork for new shortcuts. Empty = no artwork.
    pub steamgriddb_key: String,
    /// Look for a newer release on GitHub when the window opens.
    pub check_updates: bool,
    /// Per-game launch timings, keyed by lowercase launch code. Only games with changes appear.
    pub games: BTreeMap<String, GameSettings>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            steamgriddb_key: String::new(),
            check_updates: true,
            games: BTreeMap::new(),
        }
    }
}

/// Launch timings for one game; `None` = the launcher's default. They end up in the shortcut's
/// launch options, so they work the same whether the shortcut was made here or edited in Steam.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    pub startup_wait: Option<u64>,
    pub retry_after: Option<u64>,
    pub settle_delay: Option<u64>,
    pub relaunch_grace: Option<u64>,
}

impl GameSettings {
    /// (`run` option, value) for every timing that's set, in a fixed order.
    pub fn args(&self) -> Vec<(&'static str, u64)> {
        [
            ("--startup-wait", self.startup_wait),
            ("--retry-after", self.retry_after),
            ("--settle-delay", self.settle_delay),
            ("--relaunch-grace", self.relaunch_grace),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
    }

    pub fn is_default(&self) -> bool {
        *self == GameSettings::default()
    }
}

pub fn dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(crate::EXE_STEM))
}

impl Config {
    pub fn load() -> Self {
        dir()
            .and_then(|d| std::fs::read(d.join("config.json")).ok())
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = dir().ok_or_else(|| anyhow::anyhow!("no settings folder"))?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("config.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn game(&self, code: &str) -> GameSettings {
        self.games
            .get(&code.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_game(&mut self, code: &str, settings: GameSettings) {
        let key = code.to_ascii_lowercase();
        if settings.is_default() {
            self.games.remove(&key);
        } else {
            self.games.insert(key, settings);
        }
    }
}
