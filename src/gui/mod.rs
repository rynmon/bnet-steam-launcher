//! The setup window: tick the Battle.net games you want in Steam, then save.

mod jobs;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;

use crate::bnet::BnetEnv;
use crate::config::Config;
use crate::setup::{self, GameRow};
use crate::steam::{Account, ShortcutInfo, Shortcuts, SteamInstall};
use crate::update::Release;

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
                eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png"))
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
    /// A newer release on GitHub.
    newer: Option<Release>,
    update_check: Option<Receiver<Option<Release>>>,
    /// Installing `newer`: the installed copy's path once it's in place.
    updating: Option<Receiver<Result<PathBuf, String>>>,
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
            updating: None,
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
                Action::Save => jobs::save(&jobs::SaveJob {
                    steam: &steam,
                    account,
                    env,
                    pending: &pending,
                    setting,
                    artwork_key: &key,
                    restart_steam,
                }),
                Action::InstallBnet => jobs::install_bnet(&steam, account, restart_steam),
                Action::Uninstall => jobs::uninstall(&steam, account, env, restart_steam),
            };
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.working = Some(rx);
        self.result = None;
    }

    /// Downloads and installs the newer release in the background; `poll` restarts into it.
    fn start_update(&mut self) {
        let Some(release) = self.newer.clone() else {
            return;
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::update::install(&release).map_err(|e| format!("{e:#}")));
        });
        self.updating = Some(rx);
        self.result = None;
    }

    /// Picks up finished background work, and keeps the window redrawing while any is running
    /// (it only redraws on input otherwise, so a result could sit unseen).
    fn poll(&mut self, ctx: &egui::Context) {
        let mut busy = false;
        if let Some(rx) = &self.working {
            match rx.try_recv() {
                Ok(result) => {
                    self.working = None;
                    self.result = Some(result);
                    self.detect();
                }
                Err(TryRecvError::Empty) => busy = true,
                Err(TryRecvError::Disconnected) => self.working = None,
            }
        }
        if let Some(rx) = &self.update_check {
            match rx.try_recv() {
                Ok(newer) => {
                    self.newer = newer;
                    self.update_check = None;
                }
                Err(TryRecvError::Empty) => busy = true,
                Err(TryRecvError::Disconnected) => self.update_check = None,
            }
        }
        if let Some(rx) = &self.updating {
            match rx.try_recv() {
                Ok(Ok(exe)) => {
                    self.updating = None;
                    // The shortcuts already run the installed copy, so they use the new version
                    // from now on. Open it in place of this window.
                    match std::process::Command::new(&exe).spawn() {
                        Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => {
                            self.result = Some(Err(format!(
                                "Updated, but couldn't open the new version ({e}): open it from \
                                 the Start menu / application menu."
                            )))
                        }
                    }
                }
                Ok(Err(e)) => {
                    self.updating = None;
                    self.result = Some(Err(format!("Update failed: {e}")));
                }
                Err(TryRecvError::Empty) => busy = true,
                Err(TryRecvError::Disconnected) => self.updating = None,
            }
        }
        if busy {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll(ui.ctx());

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
            self.update_notice(ui);
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
