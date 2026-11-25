//! Steam side: finding Steam, its accounts, reading/writing non-Steam shortcuts, and (on Linux)
//! the per-shortcut Proton setting.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::vdf_bin::{self, Map, Value, str_val};
use crate::vdf_text::{self, Node};

#[derive(Clone, Debug)]
pub struct SteamInstall {
    pub root: PathBuf,
    /// Linux: Steam installed as a Flatpak (it's started and stopped through `flatpak run`).
    pub flatpak: bool,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub id: u32,
    pub name: String,
    pub most_recent: bool,
    /// Last sign-in time (Unix seconds), from loginusers.vdf; 0 if unknown.
    pub last_login: u64,
    pub dir: PathBuf,
}

impl Account {
    pub fn shortcuts_path(&self) -> PathBuf {
        self.dir.join("config").join("shortcuts.vdf")
    }
}

impl SteamInstall {
    #[cfg(windows)]
    pub fn find() -> Option<Self> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let mut candidates = Vec::new();
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam")
            && let Ok(p) = k.get_value::<String, _>("SteamPath")
        {
            candidates.push(PathBuf::from(p));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            candidates.push(PathBuf::from(pf).join("Steam"));
        }
        candidates
            .into_iter()
            .find(|p| p.join("steam.exe").is_file())
            .map(|root| SteamInstall {
                root,
                flatpak: false,
            })
    }

    #[cfg(not(windows))]
    pub fn find() -> Option<Self> {
        let home = dirs::home_dir()?;
        let native = [home.join(".steam/steam"), home.join(".local/share/Steam")];
        if let Some(root) = native.into_iter().find(|p| p.join("userdata").is_dir()) {
            let root = root.canonicalize().unwrap_or(root);
            return Some(SteamInstall {
                root,
                flatpak: false,
            });
        }
        let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        flatpak.join("userdata").is_dir().then_some(SteamInstall {
            root: flatpak,
            flatpak: true,
        })
    }

    /// Every Steam library folder (the main install first). Used to find Proton prefixes on Linux.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn library_folders(&self) -> Vec<PathBuf> {
        let mut out = vec![self.root.clone()];
        let vdf = self.root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(vdf)
            && let Ok(node) = vdf_text::parse(&text)
            && let Some(libs) = node.get("libraryfolders")
        {
            for (_, lib) in libs.entries() {
                if let Some(path) = lib.get("path").and_then(Node::as_str) {
                    let path = PathBuf::from(path);
                    if !out.iter().any(|p| same_path(p, &path)) {
                        out.push(path);
                    }
                }
            }
        }
        out
    }

    pub fn accounts(&self) -> Vec<Account> {
        let users = std::fs::read_to_string(self.root.join("config").join("loginusers.vdf"))
            .ok()
            .and_then(|t| vdf_text::parse(&t).ok());
        let mut accounts = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.root.join("userdata")) else {
            return accounts;
        };
        for entry in entries.flatten() {
            let Ok(id) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            if id == 0 {
                continue;
            }
            let (mut name, mut most_recent, mut last_login) = (format!("Account {id}"), false, 0);
            if let Some(users) = users.as_ref().and_then(|u| u.get("users")) {
                for (steamid64, user) in users.entries() {
                    // The account ID is the low 32 bits of the 64-bit Steam ID.
                    if steamid64
                        .parse::<u64>()
                        .ok()
                        .map(|s| (s & 0xFFFF_FFFF) as u32)
                        == Some(id)
                    {
                        if let Some(n) = user.get("PersonaName").and_then(Node::as_str) {
                            name = n.to_string();
                        }
                        most_recent = user.get("MostRecent").and_then(Node::as_str) == Some("1");
                        last_login = user
                            .get("Timestamp")
                            .and_then(Node::as_str)
                            .and_then(|t| t.parse().ok())
                            .unwrap_or(0);
                    }
                }
            }
            accounts.push(Account {
                id,
                name,
                most_recent,
                last_login,
                dir: entry.path(),
            });
        }
        // Most recently used first: that's the default choice. Steam doesn't always write
        // MostRecent, so the last sign-in time decides otherwise.
        accounts.sort_by_key(|a| (!a.most_recent, std::cmp::Reverse(a.last_login), a.id));
        accounts
    }

    pub fn config_vdf(&self) -> PathBuf {
        self.root.join("config").join("config.vdf")
    }

    /// Linux: the newest GE-Proton installed in compatibilitytools.d (it handles Battle.net best),
    /// else Valve's Proton Experimental.
    pub fn best_proton(&self) -> String {
        let mut ge: Vec<String> = std::fs::read_dir(self.root.join("compatibilitytools.d"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("GE-Proton"))
            .collect();
        ge.sort_by_key(|n| version_key(n));
        ge.pop().unwrap_or_else(|| "proton_experimental".into())
    }

    pub fn is_running() -> bool {
        let names: &[&str] = if cfg!(windows) {
            &["steam.exe"]
        } else {
            &["steam"]
        };
        crate::procs::any_named(names)
    }

    /// Asks Steam to exit and waits (up to a minute) until it has.
    pub fn shutdown(&self) -> Result<()> {
        if !Self::is_running() {
            return Ok(());
        }
        self.steam_command(&["-shutdown"])
            .status()
            .context("asking Steam to exit")?;
        let deadline = Instant::now() + Duration::from_secs(60);
        while Self::is_running() {
            if Instant::now() > deadline {
                bail!("Steam didn't exit within a minute. Close it yourself and try again.");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        // Steam writes its config files on the way out; give the last write a moment to finish.
        std::thread::sleep(Duration::from_secs(1));
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        self.start_with(&[])
    }

    /// Starts Steam with extra arguments, e.g. a `steam://` URL to run once it's up.
    pub fn start_with(&self, args: &[&str]) -> Result<()> {
        let mut cmd = self.steam_command(args);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            // Own process group, so Steam doesn't go down with this app.
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().context("starting Steam")?;
        Ok(())
    }

    fn steam_command(&self, args: &[&str]) -> std::process::Command {
        let mut cmd = if cfg!(windows) {
            std::process::Command::new(self.root.join("steam.exe"))
        } else if self.flatpak {
            let mut c = std::process::Command::new("flatpak");
            c.args(["run", "com.valvesoftware.Steam"]);
            c
        } else {
            std::process::Command::new("steam")
        };
        cmd.args(args);
        cmd
    }
}

/// "GE-Proton10-25" -> [10, 25], so versions sort numerically.
fn version_key(name: &str) -> Vec<u64> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// The `steam://rungameid/` ID of a non-Steam shortcut.
pub fn shortcut_game_id(appid: u32) -> u64 {
    (u64::from(appid) << 32) | 0x0200_0000
}

#[cfg_attr(windows, allow(dead_code))]
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

// --- shortcuts.vdf ----------------------------------------------------------------------------

/// The ID Steam uses for a non-Steam shortcut (artwork file names, Proton prefix, compat mapping).
pub fn shortcut_appid(exe: &str, name: &str) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(exe.as_bytes());
    h.update(name.as_bytes());
    h.finalize() | 0x8000_0000
}

#[derive(Clone, Debug)]
pub struct NewShortcut {
    pub name: String,
    /// Launch code, used to find a shortcut made with the old scripts for the same game.
    pub code: String,
    /// Already quoted, as Steam stores it.
    pub exe: String,
    pub start_dir: String,
    pub icon: String,
    pub launch_options: String,
    /// Default names earlier versions gave this game; an existing shortcut still called one of
    /// these is renamed to `name` (a name the user chose is left alone).
    pub old_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutInfo {
    pub appid: u32,
    pub name: String,
    pub exe: String,
    pub launch_options: String,
}

impl ShortcutInfo {
    /// Whether this shortcut was made by (and launches through) this app.
    pub fn is_ours(&self) -> bool {
        let needle = crate::EXE_STEM;
        self.exe.contains(needle) || self.launch_options.contains(needle)
    }

    /// Whether this shortcut launches through the older start-bnet-game.ps1 / .sh scripts.
    /// The "Battle.net" shortcut this app made to run Battle.net's installer under Proton.
    pub fn is_installer(&self) -> bool {
        self.exe.contains(crate::EXE_STEM) && self.exe.contains(crate::setup::INSTALLER_NAME)
    }

    pub fn is_legacy(&self) -> bool {
        !self.is_ours() && self.launch_options.contains("start-bnet-game")
    }

    /// The launch code from our launch options (`--code X`) or the old scripts' (`-LaunchCode X`,
    /// `BNET_LAUNCH_CODE=X`).
    pub fn launch_code(&self) -> Option<String> {
        let mut words = self.launch_options.split_whitespace();
        while let Some(w) = words.next() {
            if let Some(code) = w.strip_prefix("BNET_LAUNCH_CODE=") {
                return Some(code.trim_matches('"').to_string());
            }
            if w == "--code" || w.eq_ignore_ascii_case("-LaunchCode") {
                return words.next().map(|c| c.trim_matches('"').to_string());
            }
        }
        None
    }
}

pub struct Shortcuts {
    root: Map,
}

impl Shortcuts {
    /// Loads a shortcuts.vdf; a missing file is an empty list (a fresh account has none).
    pub fn load(path: &Path) -> Result<Self> {
        let root = match std::fs::read(path) {
            Ok(data) => {
                vdf_bin::parse(&data).with_context(|| format!("reading {}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut s = Shortcuts { root };
        if !matches!(s.root.get("shortcuts"), Some(Value::Map(_))) {
            s.root.set("shortcuts", Value::Map(Map::default()));
        }
        Ok(s)
    }

    fn list(&self) -> &Map {
        match self.root.get("shortcuts") {
            Some(Value::Map(m)) => m,
            _ => unreachable!("load() guarantees a shortcuts map"),
        }
    }

    fn list_mut(&mut self) -> &mut Map {
        match self.root.get_mut("shortcuts") {
            Some(Value::Map(m)) => m,
            _ => unreachable!("load() guarantees a shortcuts map"),
        }
    }

    pub fn entries(&self) -> Vec<ShortcutInfo> {
        self.list()
            .0
            .iter()
            .filter_map(|(_, v)| match v {
                Value::Map(m) => Some(info(m)),
                _ => None,
            })
            .collect()
    }

    /// Adds a shortcut, or updates one in place: ours with the same name, or one made with the
    /// old scripts for the same launch code (that one keeps its name). Updating in place keeps the
    /// shortcut's ID, so its artwork, controller layout and Proton prefix stay attached.
    /// Returns the ID.
    pub fn upsert(&mut self, s: &NewShortcut) -> u32 {
        let list = self.list_mut();
        let existing = list.0.iter_mut().find_map(|(_, v)| match v {
            Value::Map(m) => {
                let i = info(m);
                let same_code = i
                    .launch_code()
                    .is_some_and(|c| c.eq_ignore_ascii_case(&s.code));
                // Ours by name or code (the user may have renamed it), or an old script's by code.
                let same_game = (i.is_ours() && (i.name == s.name || same_code))
                    || (i.is_legacy() && same_code);
                same_game.then_some(m)
            }
            _ => None,
        });
        if let Some(m) = existing {
            if s.old_names.contains(&info(m).name) {
                m.set("AppName", str_val(&s.name));
            }
            m.set("Exe", str_val(&s.exe));
            m.set("StartDir", str_val(&s.start_dir));
            // Keep an icon the user (or Steam) already set; only fill in a missing one.
            if m.get_str("icon").unwrap_or_default().is_empty() {
                m.set("icon", str_val(&s.icon));
            }
            m.set("LaunchOptions", str_val(&s.launch_options));
            m.set("AllowOverlay", Value::Int(1));
            return info(m).appid;
        }

        let appid = shortcut_appid(&s.exe, &s.name);
        let mut m = Map::default();
        m.set("appid", Value::Int(appid as i32));
        m.set("AppName", str_val(&s.name));
        m.set("Exe", str_val(&s.exe));
        m.set("StartDir", str_val(&s.start_dir));
        m.set("icon", str_val(&s.icon));
        m.set("ShortcutPath", str_val(""));
        m.set("LaunchOptions", str_val(&s.launch_options));
        m.set("IsHidden", Value::Int(0));
        m.set("AllowDesktopConfig", Value::Int(1));
        m.set("AllowOverlay", Value::Int(1));
        m.set("OpenVR", Value::Int(0));
        m.set("Devkit", Value::Int(0));
        m.set("DevkitGameID", str_val(""));
        m.set("DevkitOverrideAppID", Value::Int(0));
        m.set("LastPlayTime", Value::Int(0));
        m.set("FlatpakAppID", str_val(""));
        m.set("sortas", str_val(""));
        let mut tags = Map::default();
        tags.0.push((b"0".to_vec(), str_val("Battle.net")));
        m.set("tags", Value::Map(tags));
        list.0.push((Vec::new(), Value::Map(m)));
        renumber(list);
        appid
    }

    /// Points an existing shortcut at a different program (keeping its ID, name and settings).
    pub fn retarget(&mut self, appid: u32, exe: &str, start_dir: &str) {
        for (_, v) in &mut self.list_mut().0 {
            if let Value::Map(m) = v
                && info(m).appid == appid
            {
                m.set("Exe", str_val(exe));
                m.set("StartDir", str_val(start_dir));
            }
        }
    }

    /// Removes one of our shortcuts by ID. Returns false if there's no such shortcut of ours.
    pub fn remove_ours(&mut self, appid: u32) -> bool {
        let list = self.list_mut();
        let before = list.0.len();
        list.0.retain(|(_, v)| !matches!(v, Value::Map(m) if { let i = info(m); i.appid == appid && i.is_ours() }));
        let removed = list.0.len() != before;
        renumber(list);
        removed
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        vdf_bin::serialize(&self.root)
    }

    /// Writes the file atomically, after keeping a timestamped backup of the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        if path.exists() {
            backup(path)?;
        } else if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(path, &self.to_bytes())
    }
}

fn info(m: &Map) -> ShortcutInfo {
    ShortcutInfo {
        appid: m.get_int("appid").unwrap_or(0) as u32,
        name: m.get_str("AppName").unwrap_or_default(),
        exe: m.get_str("Exe").unwrap_or_default(),
        launch_options: m.get_str("LaunchOptions").unwrap_or_default(),
    }
}

/// Steam numbers the entries "0", "1", ... in order.
fn renumber(list: &mut Map) {
    for (i, (k, _)) in list.0.iter_mut().enumerate() {
        *k = i.to_string().into_bytes();
    }
}

fn backup(path: &Path) -> Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let backup = path.with_file_name(format!("{name}.{}-{stamp}.bak", crate::EXE_STEM));
    std::fs::copy(path, &backup).with_context(|| format!("backing up {}", path.display()))?;
    // Keep the five most recent backups.
    if let Some(dir) = path.parent() {
        let prefix = format!("{name}.{}-", crate::EXE_STEM);
        let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
            })
            .collect();
        backups.sort();
        for old in backups.iter().rev().skip(5) {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp-bnet-launcher");
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

// --- Proton setting per shortcut (Linux) ------------------------------------------------------

const COMPAT_PATH: &[&str] = &[
    "InstallConfigStore",
    "Software",
    "Valve",
    "Steam",
    "CompatToolMapping",
];

/// The compatibility tool (Proton version) Steam has set for an app ID, if any.
pub fn compat_tool(config_vdf: &Path, appid: u32) -> Option<String> {
    let node = vdf_text::parse(&std::fs::read_to_string(config_vdf).ok()?).ok()?;
    let entry = node.get_path(COMPAT_PATH)?.get(&appid.to_string())?;
    entry
        .get("name")
        .and_then(Node::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Sets (`Some`) or clears (`None`) the compatibility tool for each given app ID.
pub fn set_compat_tools(config_vdf: &Path, changes: &[(u32, Option<String>)]) -> Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let text = std::fs::read_to_string(config_vdf)
        .with_context(|| format!("reading {}", config_vdf.display()))?;
    // Only rewrite the file if it reads back exactly: config.vdf holds all of Steam's settings, so
    // anything this parser doesn't reproduce perfectly is a reason to stop, not to guess.
    let original =
        vdf_text::parse(&text).with_context(|| format!("reading {}", config_vdf.display()))?;
    if vdf_text::serialize(&original) != text {
        bail!(
            "{} isn't in the layout this app can safely rewrite, so it was left alone.              Set each new shortcut's Proton version yourself: Properties > Compatibility.",
            config_vdf.display()
        );
    }
    let mut node =
        vdf_text::parse(&text).with_context(|| format!("reading {}", config_vdf.display()))?;
    let mut mapping = &mut node;
    for key in COMPAT_PATH {
        mapping = mapping.child_map(key);
    }
    for (appid, tool) in changes {
        match tool {
            Some(tool) => mapping.set(
                &appid.to_string(),
                Node::Map(vec![
                    ("name".into(), Node::Str(tool.clone())),
                    ("config".into(), Node::Str(String::new())),
                    ("priority".into(), Node::Str("250".into())),
                ]),
            ),
            None => mapping.remove(&appid.to_string()),
        }
    }
    backup(config_vdf)?;
    write_atomic(config_vdf, vdf_text::serialize(&node).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NewShortcut {
        NewShortcut {
            name: "WoW Forever".into(),
            code: "WoWF".into(),
            exe: "\"C:\\bnet-launcher.exe\"".into(),
            start_dir: "\"C:\\\"".into(),
            icon: String::new(),
            launch_options: "run --code WoWF --process Wow".into(),
            old_names: vec!["WoW Forever (old)".into()],
        }
    }

    #[test]
    fn proton_versions_sort_numerically() {
        let mut v = ["GE-Proton9-27", "GE-Proton10-3", "GE-Proton10-25"];
        v.sort_by_key(|n| version_key(n));
        assert_eq!(v.last(), Some(&"GE-Proton10-25"));
    }

    #[test]
    fn steam_launch_id() {
        assert_eq!(shortcut_game_id(0xC222_5DF5), 0xC222_5DF5_0200_0000);
    }

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bnet-launcher-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.vdf");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn compat_tool_written_in_steams_layout() {
        let text = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"100\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton10-25\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";
        let path = temp_file("ok", text);
        assert_eq!(compat_tool(&path, 100).as_deref(), Some("GE-Proton10-25"));
        set_compat_tools(&path, &[(200, Some("GE-Proton10-25".into()))]).unwrap();
        assert_eq!(compat_tool(&path, 200).as_deref(), Some("GE-Proton10-25"));
        set_compat_tools(&path, &[(200, None)]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "removing it restores the file exactly"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unfamiliar_config_layout_is_left_alone() {
        // Spaces instead of Steam's tabs: rewriting would change the file, so it must refuse.
        let text = "\"InstallConfigStore\" {\n  \"Software\" { }\n}\n";
        let path = temp_file("odd", text);
        assert!(set_compat_tools(&path, &[(200, Some("proton_experimental".into()))]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn upsert_and_remove() {
        let mut s = Shortcuts {
            root: Map::default(),
        };
        s.root.set("shortcuts", Value::Map(Map::default()));
        let id = s.upsert(&sample());
        assert!(id & 0x8000_0000 != 0);
        let mut changed = sample();
        changed.launch_options = "run --code WoWF --process WowB".into();
        assert_eq!(s.upsert(&changed), id, "same name updates in place");
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].launch_options, changed.launch_options);
        assert_eq!(entries[0].launch_code().as_deref(), Some("WoWF"));
        // Survives a write/read cycle.
        let again = Shortcuts {
            root: vdf_bin::parse(&s.to_bytes()).unwrap(),
        };
        assert_eq!(again.entries(), entries);
        assert!(s.remove_ours(id));
        assert!(s.entries().is_empty());
    }

    #[test]
    fn renames_only_old_default_names() {
        let mut s = Shortcuts {
            root: Map::default(),
        };
        s.root.set("shortcuts", Value::Map(Map::default()));
        let id = s.upsert(&sample()); // named "WoW Forever"

        // A new full name, but "WoW Forever" isn't listed as an old default: keep the name.
        let mut full = sample();
        full.name = "World of Warcraft: Forever".into();
        full.old_names = Vec::new();
        assert_eq!(
            s.upsert(&full),
            id,
            "matched by launch code, updated in place"
        );
        assert_eq!(s.entries()[0].name, "WoW Forever");

        // Listed as an old default: renamed, same shortcut.
        full.old_names = vec!["WoW Forever".into()];
        assert_eq!(s.upsert(&full), id);
        assert_eq!(s.entries().len(), 1);
        assert_eq!(s.entries()[0].name, "World of Warcraft: Forever");
    }

    #[test]
    fn converts_script_shortcut_in_place() {
        let mut s = Shortcuts {
            root: Map::default(),
        };
        let mut old = Map::default();
        old.set("appid", Value::Int(-1_000));
        old.set("AppName", str_val("World of Warcraft: Forever"));
        old.set("icon", str_val("c:/steam/grid/1_icon.png"));
        old.set("Exe", str_val(r#""C:\Windows\System32\conhost.exe""#));
        old.set(
            "LaunchOptions",
            str_val(r#"--headless powershell.exe -File "C:\x\start-bnet-game.ps1" -LaunchCode WoWF -GameProcess Wow"#),
        );
        let mut list = Map::default();
        list.0.push((b"0".to_vec(), Value::Map(old)));
        s.root.set("shortcuts", Value::Map(list));
        assert!(s.entries()[0].is_legacy());

        let id = s.upsert(&sample());
        assert_eq!(id, -1_000i32 as u32, "keeps the old shortcut's ID");
        let entries = s.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].name, "World of Warcraft: Forever",
            "keeps the user's name"
        );
        let Some(Value::Map(list)) = s.root.get("shortcuts") else {
            panic!()
        };
        let Value::Map(m) = &list.0[0].1 else {
            panic!()
        };
        assert_eq!(
            m.get_str("icon").as_deref(),
            Some("c:/steam/grid/1_icon.png"),
            "keeps the user's icon"
        );
        assert!(entries[0].is_ours());
    }

    #[test]
    fn never_removes_other_shortcuts() {
        let mut s = Shortcuts {
            root: Map::default(),
        };
        let mut other = Map::default();
        other.set("appid", Value::Int(5));
        other.set("AppName", str_val("Other"));
        other.set("Exe", str_val("\"C:\\other.exe\""));
        let mut list = Map::default();
        list.0.push((b"0".to_vec(), Value::Map(other)));
        s.root.set("shortcuts", Value::Map(list));
        assert!(!s.remove_ours(5));
        assert_eq!(s.entries().len(), 1);
    }
}
