//! Adding games to Steam and removing them: shared by the window and the command line.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::bnet::{self, BnetEnv};
use crate::steam::{self, NewShortcut, ShortcutInfo, Shortcuts, SteamInstall};

/// A game that can be added: from the catalog, or an installed game the catalog doesn't know.
#[derive(Clone, Debug)]
pub struct GameRow {
    pub name: String,
    pub code: String,
    pub processes: Vec<String>,
    /// Install folders (Windows form, as Battle.net stores them).
    pub game_dirs: Vec<String>,
    pub icon: Option<PathBuf>,
    pub installed: bool,
    /// "verified", "multiple", "single", or "guessed" (installed but not in the catalog).
    pub status: String,
}

impl GameRow {
    pub fn key(&self) -> String {
        self.code.to_ascii_lowercase()
    }
}

/// Every catalog game plus any installed game the catalog doesn't know; installed ones first.
pub fn game_rows(env: Option<&BnetEnv>) -> Vec<GameRow> {
    let installs = env.and_then(|e| e.installs().ok()).unwrap_or_default();
    let catalog = bnet::catalog();
    let mut rows: Vec<GameRow> = catalog
        .iter()
        .map(|game| {
            let mine: Vec<&bnet::Install> = installs.iter().filter(|i| i.matches(game)).collect();
            let mut game_dirs: Vec<String> = mine.iter().map(|i| i.install_path.clone()).collect();
            game_dirs.dedup();
            GameRow {
                name: game.name.clone(),
                code: game.code.clone(),
                processes: game.processes.clone(),
                game_dirs,
                icon: env.and_then(|e| mine.iter().find_map(|i| e.find_game_exe(i, Some(game)))),
                installed: !mine.is_empty(),
                status: game.status.clone(),
            }
        })
        .collect();
    for install in &installs {
        if catalog.iter().any(|g| install.matches(g)) {
            continue;
        }
        // Battle.net's product codes usually are the launch codes; mark it as a guess.
        rows.push(GameRow {
            name: format!("{} (not in the game list)", install.uid),
            code: install.product_code.clone(),
            processes: Vec::new(),
            game_dirs: vec![install.install_path.clone()],
            icon: None,
            installed: true,
            status: "guessed".into(),
        });
    }
    rows.sort_by_key(|r| !r.installed);
    rows
}

// --- Where the launcher lives -----------------------------------------------------------------

pub fn exe_file_name() -> String {
    format!("{}{}", crate::EXE_STEM, std::env::consts::EXE_SUFFIX)
}

/// The fixed place shortcuts point at, so they keep working if the download is moved or deleted.
pub fn installed_exe_path() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        dirs::data_local_dir()
    } else {
        dirs::data_dir()
    };
    Ok(base
        .context("no user data folder")?
        .join(crate::EXE_STEM)
        .join(exe_file_name()))
}

/// Copies this executable to `installed_exe_path()` (unless that's what's running) and returns it.
/// It also points the app's Start menu / application menu entry at that copy.
pub fn install_self() -> Result<PathBuf> {
    let target = copy_self()?;
    // Only a convenience, so a failure here mustn't stop the save.
    let _ = install_menu_entry(&target);
    Ok(target)
}

/// The name the app is listed under in the Start menu / application menu.
const MENU_NAME: &str = "Battle.net games for Steam";

/// Windows: a Start menu shortcut, so searching the Start menu finds the app. It goes in the
/// user's own Start menu folder, so no admin rights are needed.
#[cfg(windows)]
pub fn install_menu_entry(exe: &Path) -> Result<()> {
    let programs = dirs::data_dir()
        .context("no user data folder")?
        .join(r"Microsoft\Windows\Start Menu\Programs");
    std::fs::create_dir_all(&programs)?;
    let mut link = mslnk::ShellLink::new(exe)?;
    link.set_name(Some(
        "Add Battle.net games to Steam with in-game status, overlay and Steam Input".into(),
    ));
    if let Some(dir) = exe.parent() {
        link.set_working_dir(Some(dir.display().to_string()));
    }
    link.set_icon_location(Some(exe.display().to_string()));
    link.create_lnk(programs.join(format!("{MENU_NAME}.lnk")))?;
    Ok(())
}

/// Linux: installs the app icon and a `.desktop` entry in the user's own folders. Linux programs
/// can't carry an icon inside the executable like a Windows .exe; desktops find it through a
/// `.desktop` file whose name matches the window's app ID (set in gui.rs). This also puts the app
/// in the application menu.
#[cfg(not(windows))]
pub fn install_menu_entry(exe: &Path) -> Result<()> {
    let data = dirs::data_dir().context("no user data folder")?;
    let icon_dir = data.join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&icon_dir)?;
    let icon = icon_dir.join(format!("{}.png", crate::EXE_STEM));
    let png: &[u8] = include_bytes!("../assets/icon.png");
    if std::fs::read(&icon).ok().as_deref() != Some(png) {
        steam::write_atomic(&icon, png)?;
    }

    let apps = data.join("applications");
    std::fs::create_dir_all(&apps)?;
    // Exec quoting per the Desktop Entry spec: double quotes, with \ " ` $ escaped.
    let exec: String = exe
        .display()
        .to_string()
        .chars()
        .flat_map(|c| match c {
            '\\' | '"' | '`' | '$' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={MENU_NAME}\n\
         Comment=Add Battle.net games to Steam with in-game status, overlay and Steam Input\n\
         Exec=\"{exec}\"\n\
         Icon={stem}\n\
         Terminal=false\n\
         Categories=Game;Utility;\n\
         StartupWMClass={stem}\n",
        stem = crate::EXE_STEM
    );
    let path = apps.join(format!("{}.desktop", crate::EXE_STEM));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        steam::write_atomic(&path, entry.as_bytes())?;
    }
    Ok(())
}

fn copy_self() -> Result<PathBuf> {
    let target = installed_exe_path()?;
    let current = std::env::current_exe()?;
    if same_file(&current, &target) {
        return Ok(target);
    }
    let dir = target.parent().expect("has a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    if target.exists() {
        if std::fs::read(&target).ok() == std::fs::read(&current).ok() {
            return Ok(target);
        }
        // Windows can't overwrite a running exe (a game may be running through it right now),
        // but it can rename one.
        let old = target.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&target, &old)
            .with_context(|| format!("replacing {}", target.display()))?;
    }
    let tmp = target.with_extension("new");
    std::fs::copy(&current, &tmp).with_context(|| format!("copying to {}", tmp.display()))?;
    std::fs::rename(&tmp, &target).with_context(|| format!("copying to {}", target.display()))?;
    Ok(target)
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

// --- Building a shortcut ----------------------------------------------------------------------

/// Quotes one argument for Steam's launch options: Windows command-line rules there, and a POSIX
/// shell on Linux (Steam runs Linux launch options through `sh`).
pub fn quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=+,@".contains(c));
    if plain {
        arg.to_string()
    } else if cfg!(windows) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The `run ...` arguments for a game.
pub fn run_args(row: &GameRow) -> String {
    let mut parts = vec!["run".to_string(), "--code".into(), quote(&row.code)];
    for p in &row.processes {
        parts.push("--process".into());
        parts.push(quote(p));
    }
    for d in &row.game_dirs {
        parts.push("--game-dir".into());
        parts.push(quote(d));
    }
    parts.join(" ")
}

fn quoted_path(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

pub fn build_shortcut(
    row: &GameRow,
    launcher: &Path,
    env: Option<&BnetEnv>,
) -> Result<NewShortcut> {
    if cfg!(windows) {
        let dir = launcher.parent().context("launcher has no folder")?;
        Ok(NewShortcut {
            name: row.name.clone(),
            code: row.code.clone(),
            exe: quoted_path(launcher),
            start_dir: format!("\"{}\\\"", dir.display()),
            icon: row
                .icon
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            launch_options: run_args(row),
        })
    } else {
        // Linux: the target is Battle.net.exe under Proton; this launcher wraps Steam's command.
        let env = env.context("Battle.net wasn't found in any Proton prefix")?;
        let compat = env
            .compatdata
            .as_ref()
            .context("Battle.net prefix has no compatdata folder")?;
        let dir = env
            .bnet_exe
            .parent()
            .context("Battle.net.exe has no folder")?;
        Ok(NewShortcut {
            name: row.name.clone(),
            code: row.code.clone(),
            exe: quoted_path(&env.bnet_exe),
            start_dir: quoted_path(dir),
            icon: String::new(),
            // Every game shortcut shares the prefix Battle.net is installed in, instead of each
            // getting its own empty one.
            launch_options: format!(
                "STEAM_COMPAT_DATA_PATH={} {} {} -- %command%",
                quote(&compat.display().to_string()),
                quote(&launcher.display().to_string()),
                run_args(row)
            ),
        })
    }
}

/// Linux: the app ID whose compatdata folder holds the Battle.net prefix. Removing that shortcut
/// can make Steam delete the prefix, with Battle.net and every game installed in it.
pub fn prefix_owner(env: Option<&BnetEnv>) -> Option<u32> {
    env?.compatdata
        .as_ref()?
        .file_name()?
        .to_string_lossy()
        .parse()
        .ok()
}

// --- Applying changes -------------------------------------------------------------------------

/// Shortcuts that belong to a game row: ours, and ones made with the old scripts.
pub fn shortcuts_for<'a>(row: &GameRow, all: &'a [ShortcutInfo]) -> Vec<&'a ShortcutInfo> {
    all.iter()
        .filter(|s| {
            (s.is_ours() || s.is_legacy())
                && s.launch_code()
                    .is_some_and(|c| c.eq_ignore_ascii_case(&row.code))
        })
        .collect()
}

pub struct Changes<'a> {
    pub add: Vec<&'a GameRow>,
    pub remove: Vec<&'a GameRow>,
}

/// What `apply` did: a line per change for the user, and the shortcuts it added or updated
/// (app ID, name), e.g. to fetch artwork for.
#[derive(Default)]
pub struct Applied {
    pub lines: Vec<String>,
    pub added: Vec<(u32, String)>,
}

/// Writes the changes to `shortcuts_path`. For a real Steam account (`real_account`), Steam must
/// not be running (it would overwrite the changes when it exits), and on Linux the Proton setting
/// of each shortcut is updated too. `launcher` overrides where shortcuts point (tests only);
/// normally this executable is installed to its fixed place. Returns a line per change.
pub fn apply(
    steam: &SteamInstall,
    shortcuts_path: &Path,
    env: Option<&BnetEnv>,
    changes: &Changes,
    launcher: Option<&Path>,
    real_account: bool,
) -> Result<Applied> {
    if real_account && SteamInstall::is_running() {
        bail!("Steam is running. Close it first, or it will overwrite the changes when it exits.");
    }
    let path = shortcuts_path;
    let mut shortcuts = Shortcuts::load(path)?;
    let mut report = Vec::new();
    let mut compat_changes: Vec<(u32, Option<String>)> = Vec::new();
    let mut added = Vec::new();
    let owner = prefix_owner(env);

    // Linux: once Battle.net's installer (run through the shortcut this app made) has installed
    // Battle.net into that shortcut's prefix, point the shortcut at Battle.net itself.
    if let (Some(env), Some(owner)) = (env, owner)
        && shortcuts
            .entries()
            .iter()
            .any(|s| s.appid == owner && s.is_installer())
    {
        let dir = env
            .bnet_exe
            .parent()
            .context("Battle.net.exe has no folder")?;
        shortcuts.retarget(owner, &quoted_path(&env.bnet_exe), &quoted_path(dir));
        report.push(
            "The Battle.net shortcut now opens Battle.net instead of its installer".to_string(),
        );
    }

    if !changes.add.is_empty() {
        let launcher = match launcher {
            Some(l) => l.to_path_buf(),
            None => install_self()?,
        };
        // Linux: new shortcuts use the same Proton version as the one Battle.net was installed with.
        let tool = owner
            .and_then(|o| steam::compat_tool(&steam.config_vdf(), o))
            .unwrap_or_else(|| steam.best_proton());
        for row in &changes.add {
            let before = shortcuts.entries();
            let converted = shortcuts_for(row, &before).iter().any(|s| s.is_legacy());
            let shortcut = build_shortcut(row, &launcher, env)?;
            let appid = shortcuts.upsert(&shortcut);
            added.push((appid, row.name.clone()));
            if !cfg!(windows) && steam::compat_tool(&steam.config_vdf(), appid).is_none() {
                compat_changes.push((appid, Some(tool.clone())));
            }
            report.push(if converted {
                format!("Switched {} from the old script to the app", row.name)
            } else {
                format!("Added {}", row.name)
            });
        }
    }

    for row in &changes.remove {
        let entries = shortcuts.entries();
        for s in shortcuts_for(row, &entries)
            .into_iter()
            .filter(|s| s.is_ours())
        {
            if Some(s.appid) == owner {
                report.push(format!(
                    "Kept {}: its shortcut holds the Proton prefix Battle.net is installed in, so removing it could delete Battle.net and your games",
                    s.name
                ));
                continue;
            }
            if shortcuts.remove_ours(s.appid) {
                if !cfg!(windows) {
                    compat_changes.push((s.appid, None));
                }
                report.push(format!("Removed {}", s.name));
            }
        }
    }

    shortcuts.save(path)?;
    if real_account && !cfg!(windows) {
        steam::set_compat_tools(&steam.config_vdf(), &compat_changes)?;
    }
    Ok(Applied {
        lines: report,
        added,
    })
}

// --- Installing Battle.net ----------------------------------------------------------------------

/// Battle.net's official installer (redirects to the current Battle.net-Setup.exe).
const INSTALLER_URL: &str = "https://www.battle.net/download/getInstallerForGame?os=win&gameProgram=BATTLENET_APP&version=Live";
pub const INSTALLER_NAME: &str = "Battle.net-Setup.exe";

fn download_installer() -> Result<PathBuf> {
    let dir = installed_exe_path()?
        .parent()
        .expect("has a parent")
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(INSTALLER_NAME);
    crate::net::download(INSTALLER_URL, &path, &[])?;
    Ok(path)
}

/// Windows: downloads Battle.net's installer and starts it.
pub fn install_bnet_windows() -> Result<String> {
    let installer = download_installer()?;
    std::process::Command::new(&installer)
        .spawn()
        .with_context(|| format!("starting {}", installer.display()))?;
    Ok("Battle.net's installer is open. Install Battle.net, sign in and install your games, then press Refresh.".into())
}

/// Linux: downloads Battle.net's installer and adds it to Steam as a "Battle.net" shortcut running
/// under Proton, so Battle.net gets installed into that shortcut's prefix. Steam must be closed.
/// Returns the shortcut's app ID, to start it with once Steam is back.
pub fn add_bnet_installer(steam: &SteamInstall, shortcuts_path: &Path) -> Result<u32> {
    if SteamInstall::is_running() {
        bail!("Steam is running. Close it first, or it will overwrite the changes when it exits.");
    }
    let installer = download_installer()?;
    let mut shortcuts = Shortcuts::load(shortcuts_path)?;
    let appid = shortcuts.upsert(&NewShortcut {
        name: "Battle.net".into(),
        code: String::new(),
        exe: quoted_path(&installer),
        start_dir: quoted_path(installer.parent().expect("has a parent")),
        icon: String::new(),
        launch_options: String::new(),
    });
    shortcuts.save(shortcuts_path)?;
    steam::set_compat_tools(&steam.config_vdf(), &[(appid, Some(steam.best_proton()))])?;
    Ok(appid)
}

/// Whether Battle.net is running (it rewrites its settings file when it exits).
pub fn bnet_running() -> bool {
    crate::procs::snapshot()
        .iter()
        .any(|p| p.name == "battle.net.exe")
}

/// Linux Game Mode: Steam *is* the session there, so it can't be closed and restarted.
pub fn in_game_mode() -> bool {
    !cfg!(windows)
        && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.eq_ignore_ascii_case("gamescope"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options() {
        let row = GameRow {
            name: "WoW Forever".into(),
            code: "WoWF".into(),
            processes: vec!["Wow".into(), "WowB".into()],
            game_dirs: vec!["O:/Blizzard/World of Warcraft".into()],
            icon: None,
            installed: true,
            status: "verified".into(),
        };
        let q = if cfg!(windows) { "\"" } else { "'" };
        assert_eq!(
            run_args(&row),
            format!(
                "run --code WoWF --process Wow --process WowB --game-dir {q}O:/Blizzard/World of Warcraft{q}"
            )
        );
    }
}
