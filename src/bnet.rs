//! Battle.net side: the game catalog, the installed-games database (`product.db`), and finding
//! Battle.net itself.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct CatalogGame {
    pub name: String,
    /// Code for Battle.net's `--exec="launch <code>"`.
    pub code: String,
    /// Battle.net product IDs (uid or product code in product.db) that mean this game is installed.
    pub products: Vec<String>,
    /// Known process names (without .exe). Optional: the install folder is used as well.
    pub processes: Vec<String>,
    /// "verified", "multiple" or "single" (how many sources back the launch code).
    pub status: String,
}

pub fn catalog() -> Vec<CatalogGame> {
    serde_json::from_str(include_str!("../games.json")).expect("games.json is valid")
}

/// One entry of Battle.net's installed-products database.
#[derive(Clone, Debug, PartialEq)]
pub struct Install {
    pub uid: String,
    pub product_code: String,
    /// As Battle.net stores it: a Windows path with forward slashes, e.g. "O:/Blizzard/World of Warcraft".
    pub install_path: String,
    /// Version folder inside the install path, e.g. "_forever_" (empty for most games).
    pub subfolder: String,
}

/// Battle.net's own components, which are in product.db but aren't games.
const NOT_GAMES: &[&str] = &["agent", "battle.net", "bna", "bdk", "bdko"];

impl Install {
    pub fn is_game(&self) -> bool {
        !NOT_GAMES.contains(&self.uid.to_ascii_lowercase().as_str())
            && !NOT_GAMES.contains(&self.product_code.to_ascii_lowercase().as_str())
            && !self.install_path.is_empty()
    }

    pub fn matches(&self, game: &CatalogGame) -> bool {
        let ids = [
            self.uid.to_ascii_lowercase(),
            self.product_code.to_ascii_lowercase(),
        ];
        ids.iter().any(|id| {
            game.products.iter().any(|p| p.eq_ignore_ascii_case(id))
                || game.code.eq_ignore_ascii_case(id)
        })
    }
}

// --- product.db -------------------------------------------------------------------------------
//
// product.db is a protobuf message. Only the fields needed here are read, straight from the wire
// format, so no schema or protobuf dependency is needed:
//   1 (repeated) ProductInstall { 1 uid, 2 product_code, 3 Settings { 1 install_path, 13 subfolder } }

fn varint(data: &[u8], pos: &mut usize) -> Result<u64> {
    let mut result = 0u64;
    for shift in (0..64).step_by(7) {
        let Some(&b) = data.get(*pos) else {
            bail!("truncated varint")
        };
        *pos += 1;
        result |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Ok(result);
        }
    }
    bail!("varint too long")
}

/// Iterates (field number, length-delimited payload) pairs; other wire types are skipped.
fn fields(data: &[u8]) -> Result<Vec<(u64, &[u8])>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let key = varint(data, &mut pos)?;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                varint(data, &mut pos)?;
            }
            1 => pos += 8,
            5 => pos += 4,
            2 => {
                let len = usize::try_from(varint(data, &mut pos)?)?;
                let end = pos
                    .checked_add(len)
                    .filter(|&e| e <= data.len())
                    .context("truncated field")?;
                out.push((field, &data[pos..end]));
                pos = end;
            }
            other => bail!("unsupported wire type {other}"),
        }
        if pos > data.len() {
            bail!("truncated field");
        }
    }
    Ok(out)
}

fn string_field(data: &[u8], field: u64) -> Result<String> {
    Ok(fields(data)?
        .into_iter()
        .find(|(f, _)| *f == field)
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default())
}

pub fn parse_product_db(data: &[u8]) -> Result<Vec<Install>> {
    let mut installs = Vec::new();
    for (field, install) in fields(data)? {
        if field != 1 {
            continue;
        }
        let mut entry = Install {
            uid: string_field(install, 1)?,
            product_code: string_field(install, 2)?,
            install_path: String::new(),
            subfolder: String::new(),
        };
        if let Some((_, settings)) = fields(install)?.into_iter().find(|(f, _)| *f == 3) {
            entry.install_path = string_field(settings, 1)?;
            entry.subfolder = string_field(settings, 13)?;
        }
        installs.push(entry);
    }
    Ok(installs)
}

pub fn read_product_db(path: &Path) -> Result<Vec<Install>> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse_product_db(&data).with_context(|| format!("reading {}", path.display()))
}

// --- Battle.net's own settings ----------------------------------------------------------------

/// Battle.net.config key for Settings > App > "When I launch a game".
const LAUNCH_BEHAVIOR: &str = "GameLaunchWindowBehavior";
/// Its value for "Exit Battle.net completely".
const EXIT_COMPLETELY: &str = "2";

/// JSON in Battle.net's own layout: 4-space indents, `\/` for `/`, Windows line endings, and a
/// final line break.
fn battlenet_json(json: &serde_json::Value) -> Result<Vec<u8>> {
    use serde::Serialize;
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    json.serialize(&mut serde_json::Serializer::with_formatter(&mut out, fmt))?;
    // `/` only ever appears inside strings in JSON, so escaping every one is safe.
    let text = String::from_utf8(out)?
        .replace('/', r"\/")
        .replace('\n', "\r\n");
    Ok(format!("{text}\r\n").into_bytes())
}

// --- Where Battle.net lives -------------------------------------------------------------------

/// A Battle.net installation: native on Windows, or inside a Proton prefix on Linux.
#[derive(Clone, Debug)]
pub struct BnetEnv {
    pub bnet_exe: PathBuf,
    pub product_db: PathBuf,
    /// Linux: the Steam compatdata folder holding the prefix (passed as STEAM_COMPAT_DATA_PATH).
    pub compatdata: Option<PathBuf>,
}

impl BnetEnv {
    /// Turns a Windows path from product.db into a path this OS can open.
    pub fn host_path(&self, windows_path: &str) -> PathBuf {
        match &self.compatdata {
            None => PathBuf::from(windows_path),
            Some(compat) => {
                // C:/foo -> <prefix>/dosdevices/c:/foo (dosdevices maps every Wine drive letter).
                let (drive, rest) =
                    windows_path.split_at(windows_path.find(':').map_or(0, |i| i + 1));
                let mut p = compat
                    .join("pfx")
                    .join("dosdevices")
                    .join(drive.to_ascii_lowercase());
                for part in rest.split(['/', '\\']).filter(|s| !s.is_empty()) {
                    p.push(part);
                }
                p
            }
        }
    }

    /// Battle.net's own settings file (`%APPDATA%\Battle.net\Battle.net.config`).
    pub fn config_path(&self) -> Option<PathBuf> {
        match &self.compatdata {
            None => Some(
                dirs::config_dir()?
                    .join("Battle.net")
                    .join("Battle.net.config"),
            ),
            Some(compat) => {
                Some(compat.join(
                    "pfx/drive_c/users/steamuser/AppData/Roaming/Battle.net/Battle.net.config",
                ))
            }
        }
    }

    /// Whether Battle.net's "When I launch a game" setting is "Exit Battle.net completely".
    /// None if Battle.net hasn't written its settings yet (it hasn't been run and signed in).
    pub fn exits_on_launch(&self) -> Option<bool> {
        let data = std::fs::read(self.config_path()?).ok()?;
        let json: serde_json::Value = serde_json::from_slice(&data).ok()?;
        Some(json["Client"][LAUNCH_BEHAVIOR].as_str() == Some(EXIT_COMPLETELY))
    }

    /// Changes that setting: "Exit Battle.net completely", or back to Battle.net's default
    /// (keep it open). Battle.net must be closed, or it overwrites the file when it exits.
    pub fn set_exits_on_launch(&self, exit: bool) -> Result<()> {
        let path = self
            .config_path()
            .context("no Battle.net settings folder")?;
        let data = std::fs::read(&path).with_context(|| {
            format!(
                "reading {} - open Battle.net and sign in once first",
                path.display()
            )
        })?;
        let mut json: serde_json::Value =
            serde_json::from_slice(&data).with_context(|| format!("reading {}", path.display()))?;
        let client = json
            .as_object_mut()
            .context("unexpected Battle.net settings format")?
            .entry("Client")
            .or_insert_with(|| serde_json::json!({}));
        let client = client
            .as_object_mut()
            .context("unexpected Battle.net settings format")?;
        if exit {
            client.insert(LAUNCH_BEHAVIOR.into(), EXIT_COMPLETELY.into());
        } else {
            client.remove(LAUNCH_BEHAVIOR);
        }
        std::fs::copy(&path, path.with_extension("config.bnet-launcher.bak"))?;
        crate::steam::write_atomic(&path, &battlenet_json(&json)?)
    }

    pub fn installs(&self) -> Result<Vec<Install>> {
        Ok(read_product_db(&self.product_db)?
            .into_iter()
            .filter(Install::is_game)
            .collect())
    }

    /// Best guess at the game's own exe, used as the Steam shortcut's icon.
    pub fn find_game_exe(&self, install: &Install, game: Option<&CatalogGame>) -> Option<PathBuf> {
        let mut root = self.host_path(&install.install_path);
        if !install.subfolder.is_empty() {
            root.push(&install.subfolder);
        }
        let names: Vec<String> = game
            .map(|g| {
                g.processes
                    .iter()
                    .map(|p| format!("{}.exe", p.to_ascii_lowercase()))
                    .collect()
            })
            .unwrap_or_default();
        // Shallow search: games keep their exe at most a couple of folders down.
        let mut dirs = vec![(root, 0)];
        while let Some((dir, depth)) = dirs.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if path.is_dir() && depth < 2 {
                    dirs.push((path, depth + 1));
                } else if names.contains(&name) {
                    return Some(path);
                }
            }
        }
        None
    }
}

#[cfg(windows)]
pub fn find_bnet() -> Vec<BnetEnv> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for key in [
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Battle.net",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Battle.net",
        ] {
            if let Ok(k) = hklm.open_subkey(key)
                && let Ok(loc) = k.get_value::<String, _>("InstallLocation")
            {
                candidates.push(PathBuf::from(loc).join("Battle.net.exe"));
            }
        }
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(pf) = std::env::var_os(var) {
            candidates.push(PathBuf::from(pf).join("Battle.net").join("Battle.net.exe"));
        }
    }
    let program_data = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .map(|bnet_exe| BnetEnv {
            bnet_exe,
            product_db: program_data
                .join("Battle.net")
                .join("Agent")
                .join("product.db"),
            compatdata: None,
        })
        .into_iter()
        .collect()
}

/// Linux: every Proton prefix (in any Steam library) that has Battle.net installed.
#[cfg(not(windows))]
pub fn find_bnet() -> Vec<BnetEnv> {
    let Some(steam) = crate::steam::SteamInstall::find() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for lib in steam.library_folders() {
        let Ok(entries) = std::fs::read_dir(lib.join("steamapps").join("compatdata")) else {
            continue;
        };
        for entry in entries.flatten() {
            let compat = entry.path();
            let drive_c = compat.join("pfx").join("drive_c");
            let bnet_exe = drive_c
                .join("Program Files (x86)")
                .join("Battle.net")
                .join("Battle.net.exe");
            if bnet_exe.is_file() {
                out.push(BnetEnv {
                    bnet_exe,
                    product_db: drive_c
                        .join("ProgramData")
                        .join("Battle.net")
                        .join("Agent")
                        .join("product.db"),
                    compatdata: Some(compat),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ld(field: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![(field << 3) | 2, u8::try_from(payload.len()).unwrap()];
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn parses_installs() {
        let mut settings = ld(1, b"O:/Blizzard/World of Warcraft");
        settings.extend([3 << 3, 0]); // a varint field, skipped
        settings.extend(ld(13, b"_forever_"));
        let mut install = ld(1, b"wowf");
        install.extend(ld(2, b"wowf"));
        install.extend(ld(3, &settings));
        let mut db = ld(1, &install);
        db.extend(ld(1, &[ld(1, b"agent"), ld(2, b"agent")].concat()));

        let installs = parse_product_db(&db).unwrap();
        assert_eq!(installs.len(), 2);
        assert_eq!(installs[0].install_path, "O:/Blizzard/World of Warcraft");
        assert_eq!(installs[0].subfolder, "_forever_");
        assert!(installs[0].is_game());
        assert!(!installs[1].is_game());
        let wowf = catalog().into_iter().find(|g| g.code == "WoWF").unwrap();
        assert!(installs[0].matches(&wowf));
    }

    fn env_in(dir: &Path) -> BnetEnv {
        BnetEnv {
            bnet_exe: PathBuf::new(),
            product_db: PathBuf::new(),
            compatdata: Some(dir.to_path_buf()),
        }
    }

    #[test]
    fn exit_on_launch_setting() {
        let dir = std::env::temp_dir().join(format!("bnet-launcher-test-{}", std::process::id()));
        let env = env_in(&dir);
        let path = env.config_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "{\r\n    \"Client\": {\r\n        \"Version\": \"1\",\r\n        \"Zebra\": \"z:\\/x\"\r\n    },\r\n    \"Games\": {}\r\n}\r\n";
        std::fs::write(&path, original).unwrap();

        assert_eq!(env.exits_on_launch(), Some(false));
        env.set_exits_on_launch(true).unwrap();
        assert_eq!(env.exits_on_launch(), Some(true));
        let written = std::fs::read_to_string(&path).unwrap();
        // Other settings, their order, and Battle.net's layout are kept.
        assert!(written.starts_with(
            "{\r\n    \"Client\": {\r\n        \"Version\": \"1\",\r\n        \"Zebra\""
        ));
        env.set_exits_on_launch(false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Battle.net's real settings file is rewritten byte-for-byte when nothing changes:
    /// BNET_TEST_BNET_CONFIG=<path to a copy> cargo test -- --ignored
    #[test]
    #[ignore]
    fn real_bnet_config_roundtrip() {
        let path = std::env::var("BNET_TEST_BNET_CONFIG").expect("set BNET_TEST_BNET_CONFIG");
        let data = std::fs::read(path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(
            String::from_utf8(battlenet_json(&json).unwrap()).unwrap(),
            String::from_utf8(data).unwrap()
        );
    }

    /// games.json drives the app and LAUNCH_CODES.md documents it: keep them in step.
    #[test]
    fn catalog_matches_launch_codes_doc() {
        let doc = include_str!("../LAUNCH_CODES.md");
        for game in catalog() {
            assert!(
                doc.contains(&format!("| {} | `{}` |", game.name, game.code)),
                "{} ({}) missing from LAUNCH_CODES.md",
                game.name,
                game.code
            );
            assert!(
                ["verified", "multiple", "single"].contains(&game.status.as_str()),
                "{}: unknown status {}",
                game.name,
                game.status
            );
        }
    }

    // Unix only: on Windows, joining "c:" replaces the whole path, but prefixes only exist on Linux.
    #[cfg(unix)]
    #[test]
    fn host_path_in_prefix() {
        let env = BnetEnv {
            bnet_exe: PathBuf::new(),
            product_db: PathBuf::new(),
            compatdata: Some(PathBuf::from("/c")),
        };
        assert_eq!(
            env.host_path("C:/Program Files/Game"),
            PathBuf::from("/c/pfx/dosdevices/c:/Program Files/Game")
        );
    }
}
