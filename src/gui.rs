//! The setup window: tick the Battle.net games you want in Steam, then save.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, RichText};

use crate::bnet::BnetEnv;
use crate::config::Config;
use crate::setup::{self, GameRow};
use crate::steam::{Account, ShortcutInfo, Shortcuts, SteamInstall};

pub fn run() -> anyhow::Result<()> {
    // Add the app to the Start menu (Windows) or application menu (Linux, where this .desktop
    // entry is also how the window gets its icon). It points at the installed copy if there is
    // one, else at this file; saving shortcuts installs the copy and repoints it.
    let exe = setup::installed_exe_path()
        .ok()
        .filter(|p| p.is_file())
        .or_else(|| std::env::current_exe().ok());
    if let Some(exe) = exe {
        let _ = setup::install_menu_entry(&exe);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("bnet-steam-launcher")
            .with_app_id(crate::EXE_STEM)
            .with_inner_size([680.0, 680.0])
            .with_min_inner_size([480.0, 380.0])
            .with_icon(std::sync::Arc::new(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("icon.png is a valid PNG"),
            )),
        ..Default::default()
    };
    eframe::run_native(
        "bnet-launcher",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
    .map_err(|e| anyhow::anyhow!("could not open the window: {e}"))
}

/// Something that may need Steam closed, waiting on the user's OK to restart it.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Save,
    InstallBnet,
    Uninstall,
}

/// Shortcut changes waiting to be saved.
#[derive(Default)]
struct Pending {
    add: Vec<GameRow>,
    update: Vec<GameRow>,
    remove: Vec<GameRow>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.add.is_empty() && self.update.is_empty() && self.remove.is_empty()
    }
}

struct App {
    steam: Option<SteamInstall>,
    accounts: Vec<Account>,
    account: usize,
    envs: Vec<BnetEnv>,
    env: usize,
    rows: Vec<GameRow>,
    shortcuts: Vec<ShortcutInfo>,
    shortcuts_error: Option<String>,
    /// Desired state per game key: true = should be in Steam.
    wanted: HashMap<String, bool>,
    show_all: bool,
    /// The installed copy the shortcuts run is an older build than this one.
    app_outdated: bool,
    /// Battle.net's "exit when a game starts" setting: as found (None = no settings file yet),
    /// and as wanted.
    exit_on_launch: Option<bool>,
    want_exit_on_launch: bool,
    config: Config,
    artwork: bool,
    /// A newer release on GitHub: (version, page URL).
    newer: Option<(String, String)>,
    update_check: Option<Receiver<Option<(String, String)>>>,
    /// An action waiting for the user's OK, and whether Steam was running when it was asked.
    confirm: Option<(Action, bool)>,
    working: Option<Receiver<Result<Vec<String>, String>>>,
    result: Option<Result<Vec<String>, String>>,
}

impl App {
    fn new() -> Self {
        let config = Config::load();
        let update_check = config.check_updates.then(|| {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::update::newer_release().ok().flatten());
            });
            rx
        });
        let mut app = App {
            steam: None,
            accounts: Vec::new(),
            account: 0,
            envs: Vec::new(),
            env: 0,
            rows: Vec::new(),
            shortcuts: Vec::new(),
            shortcuts_error: None,
            wanted: HashMap::new(),
            show_all: false,
            app_outdated: false,
            exit_on_launch: None,
            want_exit_on_launch: false,
            artwork: !config.steamgriddb_key.is_empty(),
            config,
            newer: None,
            update_check,
            confirm: None,
            working: None,
            result: None,
        };
        app.detect();
        app
    }

    /// Finds Steam, its accounts and Battle.net again, then reloads everything else.
    fn detect(&mut self) {
        self.steam = SteamInstall::find();
        self.accounts = self
            .steam
            .as_ref()
            .map(SteamInstall::accounts)
            .unwrap_or_default();
        self.account = self.account.min(self.accounts.len().saturating_sub(1));
        self.envs = crate::bnet::find_bnet();
        self.env = self.env.min(self.envs.len().saturating_sub(1));
        self.reload();
    }

    /// Re-reads the installed games, the selected account's shortcuts and Battle.net's setting.
    fn reload(&mut self) {
        let env = self.envs.get(self.env);
        self.rows = setup::game_rows(env, &self.config);
        (self.shortcuts, self.shortcuts_error) = match self.accounts.get(self.account) {
            Some(a) => match Shortcuts::load(&a.shortcuts_path()) {
                Ok(s) => (s.entries(), None),
                Err(e) => (Vec::new(), Some(format!("{e:#}"))),
            },
            None => (Vec::new(), None),
        };
        self.wanted = self
            .rows
            .iter()
            .map(|r| (r.key(), self.has_ours(r)))
            .collect();
        self.app_outdated = setup::installed_copy_outdated();
        self.exit_on_launch = env.and_then(BnetEnv::exits_on_launch);
        self.want_exit_on_launch = self.exit_on_launch.unwrap_or(false);
    }

    fn ours(&self, row: &GameRow) -> Option<&ShortcutInfo> {
        setup::shortcuts_for(row, &self.shortcuts)
            .into_iter()
            .find(|s| s.is_ours())
    }

    fn has_ours(&self, row: &GameRow) -> bool {
        self.ours(row).is_some()
    }

    fn has_legacy(&self, row: &GameRow) -> bool {
        setup::shortcuts_for(row, &self.shortcuts)
            .iter()
            .any(|s| s.is_legacy())
    }

    /// Why a game's shortcut needs rewriting, if it does.
    fn outdated(&self, row: &GameRow) -> Option<&'static str> {
        let existing = self.ours(row)?;
        setup::outdated(row, existing, self.envs.get(self.env))
            .or(self.app_outdated.then_some("app"))
    }

    fn pending(&self) -> Pending {
        let mut p = Pending::default();
        for row in &self.rows {
            let want = self.wanted.get(&row.key()).copied().unwrap_or(false);
            match (want, self.has_ours(row)) {
                (true, false) => p.add.push(row.clone()),
                (false, true) => p.remove.push(row.clone()),
                (true, true) if self.outdated(row).is_some() => p.update.push(row.clone()),
                _ => {}
            }
        }
        p
    }

    fn setting_changed(&self) -> bool {
        self.exit_on_launch
            .is_some_and(|now| now != self.want_exit_on_launch)
    }

    /// Runs an action, asking first if it means closing Steam (and always for uninstalling).
    fn start(&mut self, action: Action) {
        let needs_steam_closed = match action {
            Action::Save => !self.pending().is_empty(),
            Action::InstallBnet => !cfg!(windows),
            Action::Uninstall => true,
        };
        let steam_running = needs_steam_closed && SteamInstall::is_running();
        if action == Action::Uninstall || steam_running {
            self.confirm = Some((action, steam_running));
        } else {
            self.run(action, false);
        }
    }

    /// Does the work in the background, so the window stays responsive while Steam restarts.
    fn run(&mut self, action: Action, restart_steam: bool) {
        let Some(steam) = self.steam.clone() else {
            return;
        };
        let account = self.accounts.get(self.account).cloned();
        let env = self.envs.get(self.env).cloned();
        let pending = self.pending();
        let setting = self.setting_changed().then_some(self.want_exit_on_launch);
        let key = if self.artwork {
            self.config.steamgriddb_key.trim().to_string()
        } else {
            String::new()
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let result = match action {
                Action::Save => save(&SaveJob {
                    steam: &steam,
                    account,
                    env,
                    pending: &pending,
                    setting,
                    artwork_key: &key,
                    restart_steam,
                }),
                Action::InstallBnet => install_bnet(&steam, account, restart_steam),
                Action::Uninstall => uninstall(&steam, account, env, restart_steam),
            };
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.working = Some(rx);
        self.result = None;
    }

    /// Everything useful for a bug report, for the clipboard. Paths can include the user's
    /// account name, so it's shown to them (pasted by them), never sent anywhere.
    fn details(&self) -> String {
        let mut out = format!(
            "bnet-steam-launcher {} on {} ({})\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            if setup::in_game_mode() {
                "Game Mode"
            } else {
                "desktop"
            }
        );
        out += &format!(
            "Steam: {}\n",
            self.steam
                .as_ref()
                .map_or("not found".into(), |s| s.root.display().to_string())
        );
        out += &format!(
            "Battle.net: {}\n",
            self.envs
                .get(self.env)
                .map_or("not found".into(), |e| e.bnet_exe.display().to_string())
        );
        out += "Installed games:";
        for r in self.rows.iter().filter(|r| r.installed) {
            out += &format!(" {} ({});", r.name, r.code);
        }
        out += "\nShortcuts made by the app:\n";
        for s in self.shortcuts.iter().filter(|s| s.is_ours()) {
            out += &format!("  {}: {}\n", s.name, s.launch_options);
        }
        let log = crate::run::log_path();
        for (title, path) in [
            ("Last launch log", log.clone()),
            ("Launch before that", log.with_extension("prev.log")),
        ] {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = text.lines().collect();
                let tail = &lines[lines.len().saturating_sub(60)..];
                out += &format!("\n{title} ({}):\n{}\n", path.display(), tail.join("\n"));
            }
        }
        out
    }
}

struct SaveJob<'a> {
    steam: &'a SteamInstall,
    account: Option<Account>,
    env: Option<BnetEnv>,
    pending: &'a Pending,
    /// Battle.net's "exit when a game starts" setting, if it should change.
    setting: Option<bool>,
    /// SteamGridDB key; empty = no artwork.
    artwork_key: &'a str,
    restart_steam: bool,
}

fn save(job: &SaveJob) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    if let (Some(exit), Some(env)) = (job.setting, &job.env) {
        if setup::bnet_running() {
            lines.push(
                "Battle.net's setting wasn't changed: close Battle.net first, then save again."
                    .into(),
            );
        } else {
            env.set_exits_on_launch(exit)?;
            lines.push(if exit {
                "Battle.net will now close itself when a game starts".into()
            } else {
                "Battle.net will now stay open when a game starts".into()
            });
        }
    }
    if job.pending.is_empty() {
        return Ok(lines);
    }
    let account = job
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no Steam account selected"))?;
    if job.restart_steam {
        job.steam.shutdown()?;
    }
    let changes = setup::Changes {
        add: job.pending.add.iter().collect(),
        update: job.pending.update.iter().collect(),
        remove: job.pending.remove.iter().collect(),
    };
    let result = setup::apply(
        job.steam,
        &account.shortcuts_path(),
        job.env.as_ref(),
        &changes,
        None,
        true,
    )
    .map(|applied| {
        lines.extend(applied.lines);
        if !job.artwork_key.is_empty() {
            let grid = account.dir.join("config").join("grid");
            for (appid, name) in &applied.added {
                lines.push(
                    match crate::artwork::fetch(name, *appid, &grid, job.artwork_key) {
                        Ok(line) => line,
                        Err(e) => format!("{name}: artwork failed: {e:#}"),
                    },
                );
            }
        }
    });
    // Start Steam again even if saving failed: it was only closed for the save.
    if job.restart_steam {
        job.steam.start()?;
    }
    result.map(|()| lines)
}

fn install_bnet(
    steam: &SteamInstall,
    account: Option<Account>,
    restart_steam: bool,
) -> anyhow::Result<Vec<String>> {
    if cfg!(windows) {
        return Ok(vec![setup::install_bnet_windows()?]);
    }
    let account = account.ok_or_else(|| anyhow::anyhow!("no Steam account selected"))?;
    if restart_steam {
        steam.shutdown()?;
    }
    let result = setup::add_bnet_installer(steam, &account.shortcuts_path());
    match &result {
        Ok(appid) => steam.start_with(&[&format!(
            "steam://rungameid/{}",
            crate::steam::shortcut_game_id(*appid)
        )])?,
        Err(_) if restart_steam => steam.start()?,
        Err(_) => {}
    }
    result?;
    Ok(vec![
        "Added a \"Battle.net\" shortcut, and Steam is starting Battle.net's installer through it."
            .into(),
        "Install Battle.net, sign in and install your games, then press Refresh here. Keep that \
         shortcut: it holds the Proton prefix everything is installed in."
            .into(),
    ])
}

fn uninstall(
    steam: &SteamInstall,
    account: Option<Account>,
    env: Option<BnetEnv>,
    restart_steam: bool,
) -> anyhow::Result<Vec<String>> {
    let account = account.ok_or_else(|| anyhow::anyhow!("no Steam account selected"))?;
    let running = restart_steam && SteamInstall::is_running();
    if running {
        steam.shutdown()?;
    }
    let result = setup::uninstall(steam, &account.shortcuts_path(), env.as_ref());
    if running {
        steam.start()?;
    }
    result
}

/// Opens a file with the system's default app.
fn open_file(path: &std::path::Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "notepad.exe"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// Default shown next to each advanced setting (what the launcher uses when it isn't set).
fn default_for(option: &str) -> u64 {
    match option {
        "--startup-wait" => 120,
        "--retry-after" | "--relaunch-grace" => 10,
        "--settle-delay" if cfg!(windows) => 3,
        _ => 8,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.working {
            match rx.try_recv() {
                Ok(result) => {
                    self.working = None;
                    self.result = Some(result);
                    self.detect();
                }
                Err(_) => ui
                    .ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200)),
            }
        }
        if let Some(rx) = &self.update_check {
            match rx.try_recv() {
                Ok(newer) => {
                    self.newer = newer;
                    self.update_check = None;
                }
                // The window only redraws on input otherwise, so the notice could sit unseen.
                Err(std::sync::mpsc::TryRecvError::Empty) => ui
                    .ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_check = None,
            }
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Battle.net games for Steam");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(self.working.is_none(), egui::Button::new("Refresh"))
                        .on_hover_text("Look for Steam, Battle.net and installed games again")
                        .clicked()
                    {
                        self.detect();
                    }
                    ui.weak(format!("v{}", env!("CARGO_PKG_VERSION")));
                });
            });
            if let Some((version, url)) = &self.newer {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(90, 180, 90), format!("Version {version} is available."));
                    ui.hyperlink_to("Download it", url);
                    ui.weak("then open it and press Save to update your shortcuts.");
                });
            }
            ui.label(
                "Tick the games you want in your Steam library. Each one gets a shortcut that starts \
                 Battle.net, launches the game, and keeps Steam's in-game status, overlay and \
                 Steam Input working.",
            );
            ui.add_space(4.0);
            self.header(ui);
            ui.add_space(6.0);
        });

        egui::Panel::bottom("bottom").show(ui, |ui| {
            ui.add_space(6.0);
            self.footer(ui);
            ui.add_space(6.0);
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    self.game_list(ui);
                    ui.add_space(8.0);
                    self.options(ui);
                });
        });

        if let Some((action, steam_running)) = self.confirm {
            self.confirm_dialog(ui, action, steam_running);
        }
    }
}

impl App {
    fn confirm_dialog(&mut self, ui: &mut egui::Ui, action: Action, steam_running: bool) {
        egui::Modal::new(egui::Id::unique("confirm")).show(ui.ctx(), |ui| {
            ui.set_max_width(400.0);
            if action == Action::Uninstall {
                ui.heading("Uninstall?");
                ui.label(
                    "This removes the Steam shortcuts this app made, its Start menu / application \
                     menu entry, its settings and its installed copy. Battle.net, your games and \
                     shortcuts you made yourself are left alone.",
                );
                if steam_running {
                    ui.label(
                        "Steam will be closed while its shortcuts change, then started again.",
                    );
                }
            } else {
                ui.heading("Restart Steam?");
                ui.label(
                    "Steam has to be closed while its shortcuts are changed, or it overwrites \
                     them. It will be started again afterwards.",
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = match (action, steam_running) {
                    (Action::Uninstall, false) => "Uninstall",
                    (Action::Uninstall, true) => "Close Steam and uninstall",
                    _ => "Close Steam and continue",
                };
                if ui.button(ok).clicked() {
                    self.confirm = None;
                    self.run(action, steam_running);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm = None;
                }
            });
        });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let Some(steam_root) = self.steam.as_ref().map(|s| s.root.display().to_string()) else {
            ui.colored_label(
                Color32::LIGHT_RED,
                "Steam wasn't found. Install Steam and sign in once, then press Refresh.",
            );
            return;
        };
        egui::Grid::new("where")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Steam");
                ui.label(steam_root);
                ui.end_row();

                ui.label("Account");
                if self.accounts.is_empty() {
                    ui.colored_label(
                        Color32::LIGHT_RED,
                        "No Steam accounts found - sign in to Steam once first.",
                    );
                } else {
                    let before = self.account;
                    egui::ComboBox::from_id_salt("account")
                        .selected_text(&self.accounts[self.account].name)
                        .show_ui(ui, |ui| {
                            for (i, a) in self.accounts.iter().enumerate() {
                                ui.selectable_value(&mut self.account, i, &a.name);
                            }
                        });
                    if before != self.account {
                        self.reload();
                    }
                }
                ui.end_row();

                ui.label("Battle.net");
                if self.envs.is_empty() {
                    ui.vertical(|ui| {
                        ui.colored_label(
                            Color32::LIGHT_RED,
                            if cfg!(windows) {
                                "Not installed."
                            } else {
                                "Not found in any Proton prefix."
                            },
                        );
                        let can = self.working.is_none()
                            && !self.accounts.is_empty()
                            && !setup::in_game_mode();
                        if ui
                            .add_enabled(can, egui::Button::new("Install Battle.net"))
                            .on_hover_text(if cfg!(windows) {
                                "Downloads Battle.net's installer from battle.net and opens it"
                            } else {
                                "Downloads Battle.net's installer from battle.net, adds it to \
                                 Steam as a \"Battle.net\" shortcut using Proton, and starts it"
                            })
                            .clicked()
                        {
                            self.start(Action::InstallBnet);
                        }
                    });
                } else if self.envs.len() == 1 {
                    ui.label(self.envs[0].bnet_exe.display().to_string());
                } else {
                    let before = self.env;
                    egui::ComboBox::from_id_salt("env")
                        .selected_text(self.envs[self.env].bnet_exe.display().to_string())
                        .show_ui(ui, |ui| {
                            for (i, e) in self.envs.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.env,
                                    i,
                                    e.bnet_exe.display().to_string(),
                                );
                            }
                        });
                    if before != self.env {
                        self.reload();
                    }
                }
                ui.end_row();
            });
        if let Some(e) = &self.shortcuts_error {
            ui.colored_label(
                Color32::LIGHT_RED,
                format!("Couldn't read this account's shortcuts: {e}"),
            );
        }
    }

    fn game_list(&mut self, ui: &mut egui::Ui) {
        // Installed games the catalog doesn't know only show with "show all": their launch code
        // is a guess, so they shouldn't look like a normal choice.
        let shown_by_default = |r: &GameRow| r.installed && r.status != "guessed";
        let installed = self.rows.iter().filter(|r| shown_by_default(r)).count();
        if installed == 0 && !self.envs.is_empty() {
            ui.label(
                "No installed games found in Battle.net. Install one there and press Refresh, \
                 or pick from the full list below.",
            );
            self.show_all = true;
        }
        let indices: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                self.show_all || shown_by_default(r) || self.has_ours(r)
            })
            .collect();
        for i in indices {
            let row = self.rows[i].clone();
            let key = row.key();
            let (ours, legacy) = (self.ours(&row).cloned(), self.has_legacy(&row));
            let outdated = self.outdated(&row);
            let mut want = self.wanted.get(&key).copied().unwrap_or(false);
            ui.horizontal(|ui| {
                ui.checkbox(&mut want, RichText::new(&row.name).strong());
                ui.weak(&row.code);
                let (text, color, hint) = match row.status.as_str() {
                    "verified" => (
                        "tested",
                        Color32::from_rgb(90, 180, 90),
                        "Tested with this app",
                    ),
                    "guessed" => (
                        "launch code guessed",
                        Color32::from_rgb(220, 160, 60),
                        "Not in the app's game list: Battle.net's product code is used as the \
                         launch code, which may not work",
                    ),
                    _ => (
                        "untested",
                        Color32::GRAY,
                        "Launch code from community sources, not tested with this app",
                    ),
                };
                ui.colored_label(color, text).on_hover_text(hint);
            });
            let note = match (ours.is_some(), want, legacy, row.installed) {
                (true, true, _, _) => Some(match (outdated, &ours) {
                    (Some("rename"), Some(s)) => format!("In Steam - will be renamed from \"{}\"", s.name),
                    (Some("settings"), _) => "In Steam - will be updated with the new settings".to_string(),
                    (Some(_), _) => "In Steam - will be updated to this version of the app".to_string(),
                    _ => "In Steam".to_string(),
                }),
                (true, false, _, _) => Some("Will be removed from Steam".into()),
                (false, true, true, _) => Some(
                    "Will replace the shortcut made with the old script (keeping its name and artwork)"
                        .into(),
                ),
                (false, false, true, _) => {
                    Some("In Steam through the old script - tick to switch it to this app".into())
                }
                (false, true, false, false) => {
                    Some("Not detected as installed - it will be added anyway".into())
                }
                _ => None,
            };
            ui.indent(&key, |ui| {
                if let Some(note) = note {
                    ui.weak(note);
                }
                if want {
                    self.advanced(ui, i);
                }
            });
            self.wanted.insert(key, want);
            ui.add_space(2.0);
        }
        ui.add_space(6.0);
        if installed > 0 {
            ui.checkbox(
                &mut self.show_all,
                "Show games that aren't installed or aren't in the game list",
            );
        }
    }

    /// Per-game launch timings. They're written into the shortcut's launch options on save.
    fn advanced(&mut self, ui: &mut egui::Ui, i: usize) {
        let code = self.rows[i].code.clone();
        egui::CollapsingHeader::new(RichText::new("Advanced").small())
            .id_salt(("advanced", &code))
            .show(ui, |ui| {
                let mut s = self.rows[i].settings.clone();
                let fields: [(&str, &str, &mut Option<u64>); 4] = [
                    (
                        "--startup-wait",
                        "Give up if the game hasn't started after (seconds)",
                        &mut s.startup_wait,
                    ),
                    (
                        "--retry-after",
                        "Resend the launch command every (seconds)",
                        &mut s.retry_after,
                    ),
                    (
                        "--settle-delay",
                        "Wait after Battle.net opens before launching (seconds)",
                        &mut s.settle_delay,
                    ),
                    (
                        "--relaunch-grace",
                        "Keep watching after the game exits, in case it restarts (seconds)",
                        &mut s.relaunch_grace,
                    ),
                ];
                egui::Grid::new(("advanced-grid", &code))
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (option, label, value) in fields {
                            let default = default_for(option);
                            let mut custom = value.is_some();
                            ui.checkbox(&mut custom, label);
                            if custom {
                                let mut v = value.unwrap_or(default);
                                ui.add(egui::DragValue::new(&mut v).range(0..=3600));
                                *value = Some(v);
                            } else {
                                ui.weak(format!("{default} (default)"));
                                *value = None;
                            }
                            ui.end_row();
                        }
                    });
                ui.weak("Raise the startup wait for games that take a long time to open, or the settle delay if Battle.net ignores the first launch.");
                if s != self.rows[i].settings {
                    self.rows[i].settings = s.clone();
                    self.config.set_game(&code, s);
                    if let Err(e) = self.config.save() {
                        self.result = Some(Err(format!("Couldn't save the settings: {e:#}")));
                    }
                }
            });
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Options").show(ui, |ui| {
            let label = "Close Battle.net when a game starts";
            if self.exit_on_launch.is_some() {
                ui.checkbox(&mut self.want_exit_on_launch, label).on_hover_text(
                    "Battle.net's own \"When I launch a game\" setting. Recommended on Steam \
                     Deck, where Battle.net's window can otherwise keep the controller while \
                     you play.",
                );
            } else {
                ui.add_enabled(false, egui::Checkbox::new(&mut false, label))
                    .on_disabled_hover_text(
                        "Open Battle.net and sign in once first, so it has a settings file.",
                    );
            }
            if !cfg!(windows) && self.exit_on_launch == Some(false) {
                ui.weak(
                    "Recommended on Steam Deck: otherwise Battle.net's window can keep the \
                     controller while you play.",
                );
            }
            ui.add_space(6.0);

            ui.checkbox(
                &mut self.artwork,
                "Download artwork for new shortcuts from SteamGridDB",
            );
            ui.horizontal(|ui| {
                ui.label("API key");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.config.steamgriddb_key)
                        .password(true)
                        .desired_width(260.0),
                );
                if edit.lost_focus() {
                    if let Err(e) = self.config.save() {
                        self.result = Some(Err(format!("Couldn't save the API key: {e:#}")));
                    }
                    if !self.config.steamgriddb_key.trim().is_empty() {
                        self.artwork = true;
                    }
                }
                ui.hyperlink_to(
                    "Get a free key",
                    "https://www.steamgriddb.com/profile/preferences/api",
                );
            });
            ui.weak(
                "Covers, backgrounds and logos made by the SteamGridDB community. Existing \
                 artwork is never replaced.",
            );
            ui.add_space(6.0);

            if ui
                .checkbox(&mut self.config.check_updates, "Check for new versions when this opens")
                .on_hover_text("Asks GitHub for the latest release of bnet-steam-launcher. Nothing about you is sent.")
                .changed()
                && let Err(e) = self.config.save()
            {
                self.result = Some(Err(format!("Couldn't save the setting: {e:#}")));
            }
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                let log = crate::run::log_path();
                if ui
                    .add_enabled(log.is_file(), egui::Button::new("Open launch log"))
                    .on_hover_text(log.display().to_string())
                    .on_disabled_hover_text("No game has been launched through the app yet")
                    .clicked()
                    && let Err(e) = open_file(&log)
                {
                    self.result = Some(Err(format!("Couldn't open the log: {e}")));
                }
                if ui
                    .button("Copy details for a bug report")
                    .on_hover_text("Copies the app version, what it found, and the last launch logs, to paste into a GitHub issue")
                    .clicked()
                {
                    ui.ctx().copy_text(self.details());
                    self.result = Some(Ok(vec!["Copied. Paste it into an issue at github.com/rynmon/bnet-steam-launcher/issues".into()]));
                }
                if ui
                    .add_enabled(
                        self.working.is_none() && self.steam.is_some() && !self.accounts.is_empty(),
                        egui::Button::new("Uninstall..."),
                    )
                    .clicked()
                {
                    self.start(Action::Uninstall);
                }
            });
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = &self.result {
            match result {
                Ok(lines) if lines.is_empty() => {
                    ui.label("Nothing to change.");
                }
                Ok(lines) => {
                    for l in lines {
                        ui.label(l);
                    }
                }
                Err(e) => {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
            }
            ui.separator();
        }
        let pending = self.pending();
        let setting = self.setting_changed();
        let ready = self.steam.is_some()
            && !self.accounts.is_empty()
            && (cfg!(windows) || !self.envs.is_empty());
        ui.horizontal(|ui| {
            if self.working.is_some() {
                ui.spinner();
                ui.label("Working...");
                return;
            }
            let mut parts = Vec::new();
            for (n, verb) in [
                (pending.add.len(), "add"),
                (pending.update.len(), "update"),
                (pending.remove.len(), "remove"),
            ] {
                if n > 0 {
                    parts.push(format!("{verb} {n}"));
                }
            }
            if setting {
                parts.push("change Battle.net's setting".to_string());
            }
            let summary = if parts.is_empty() {
                "No changes".to_string()
            } else {
                let s = parts.join(", ");
                s[..1].to_uppercase() + &s[1..]
            };
            let can_save = ready
                && (!pending.is_empty() || setting)
                && !(!pending.is_empty() && setup::in_game_mode());
            if ui
                .add_enabled(can_save, egui::Button::new("Save to Steam"))
                .clicked()
            {
                self.start(Action::Save);
            }
            ui.label(summary);
        });
        if setup::in_game_mode() {
            ui.colored_label(
                Color32::LIGHT_RED,
                "Switch to Desktop Mode to change shortcuts: Steam can't be restarted in Game Mode.",
            );
        } else if !pending.add.is_empty() {
            ui.weak(
                "Tip: set a controller layout in each new shortcut's Properties if you use one.",
            );
        }
    }
}
