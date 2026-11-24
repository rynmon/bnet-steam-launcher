//! The setup window: tick the Battle.net games you want in Steam, then save.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, RichText};

use crate::bnet::BnetEnv;
use crate::config::Config;
use crate::setup::{self, GameRow};
use crate::steam::{Account, ShortcutInfo, Shortcuts, SteamInstall};

pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Battle.net games for Steam")
            .with_inner_size([660.0, 640.0])
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
    /// Battle.net's "exit when a game starts" setting: as found (None = no settings file yet),
    /// and as wanted.
    exit_on_launch: Option<bool>,
    want_exit_on_launch: bool,
    config: Config,
    artwork: bool,
    confirm: Option<Action>,
    working: Option<Receiver<Result<Vec<String>, String>>>,
    result: Option<Result<Vec<String>, String>>,
}

impl App {
    fn new() -> Self {
        let config = Config::load();
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
            exit_on_launch: None,
            want_exit_on_launch: false,
            artwork: !config.steamgriddb_key.is_empty(),
            config,
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
        self.rows = setup::game_rows(env);
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
        self.exit_on_launch = env.and_then(BnetEnv::exits_on_launch);
        self.want_exit_on_launch = self.exit_on_launch.unwrap_or(false);
    }

    fn has_ours(&self, row: &GameRow) -> bool {
        setup::shortcuts_for(row, &self.shortcuts)
            .iter()
            .any(|s| s.is_ours())
    }

    fn has_legacy(&self, row: &GameRow) -> bool {
        setup::shortcuts_for(row, &self.shortcuts)
            .iter()
            .any(|s| s.is_legacy())
    }

    fn pending(&self) -> (Vec<GameRow>, Vec<GameRow>) {
        let mut add = Vec::new();
        let mut remove = Vec::new();
        for row in &self.rows {
            let want = self.wanted.get(&row.key()).copied().unwrap_or(false);
            match (want, self.has_ours(row)) {
                (true, false) => add.push(row.clone()),
                (false, true) => remove.push(row.clone()),
                _ => {}
            }
        }
        (add, remove)
    }

    fn setting_changed(&self) -> bool {
        self.exit_on_launch
            .is_some_and(|now| now != self.want_exit_on_launch)
    }

    /// Runs an action, asking first if it means closing Steam.
    fn start(&mut self, action: Action) {
        let needs_steam_closed = match action {
            Action::Save => !self.pending().0.is_empty() || !self.pending().1.is_empty(),
            Action::InstallBnet => !cfg!(windows),
        };
        if needs_steam_closed && SteamInstall::is_running() {
            self.confirm = Some(action);
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
        let (add, remove) = self.pending();
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
                    add: &add,
                    remove: &remove,
                    setting,
                    artwork_key: &key,
                    restart_steam,
                }),
                Action::InstallBnet => install_bnet(&steam, account, restart_steam),
            };
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.working = Some(rx);
        self.result = None;
    }
}

struct SaveJob<'a> {
    steam: &'a SteamInstall,
    account: Option<Account>,
    env: Option<BnetEnv>,
    add: &'a [GameRow],
    remove: &'a [GameRow],
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
    if job.add.is_empty() && job.remove.is_empty() {
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
        add: job.add.iter().collect(),
        remove: job.remove.iter().collect(),
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
                });
            });
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

        if let Some(action) = self.confirm {
            egui::Modal::new(egui::Id::unique("restart")).show(ui.ctx(), |ui| {
                ui.set_max_width(380.0);
                ui.heading("Restart Steam?");
                ui.label(
                    "Steam has to be closed while its shortcuts are changed, or it overwrites \
                     them. It will be started again afterwards.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close Steam and continue").clicked() {
                        self.confirm = None;
                        self.run(action, true);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
        }
    }
}

impl App {
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
        let rows: Vec<GameRow> = self
            .rows
            .iter()
            .filter(|r| self.show_all || shown_by_default(r) || self.has_ours(r))
            .cloned()
            .collect();
        for row in rows {
            let key = row.key();
            let (ours, legacy) = (self.has_ours(&row), self.has_legacy(&row));
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
            let note = match (ours, want, legacy, row.installed) {
                (true, true, _, _) => Some("In Steam"),
                (true, false, _, _) => Some("Will be removed from Steam"),
                (false, true, true, _) => Some(
                    "Will replace the shortcut made with the old script (keeping its name and artwork)",
                ),
                (false, false, true, _) => {
                    Some("In Steam through the old script - tick to switch it to this app")
                }
                (false, true, false, false) => {
                    Some("Not detected as installed - it will be added anyway")
                }
                _ => None,
            };
            if let Some(note) = note {
                ui.indent(&key, |ui| ui.weak(note));
            }
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

    fn options(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Options").show(ui, |ui| {
            let label = "Close Battle.net when a game starts";
            if self.exit_on_launch.is_some() {
                ui.checkbox(&mut self.want_exit_on_launch, label)
                    .on_hover_text(
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
        let (add, remove) = self.pending();
        let setting = self.setting_changed();
        let shortcuts_change = !(add.is_empty() && remove.is_empty());
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
            if !add.is_empty() {
                parts.push(format!("add {}", add.len()));
            }
            if !remove.is_empty() {
                parts.push(format!("remove {}", remove.len()));
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
                && (shortcuts_change || setting)
                && !(shortcuts_change && setup::in_game_mode());
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
        } else if !add.is_empty() {
            ui.weak(
                "Tip: set a controller layout in each new shortcut's Properties if you use one.",
            );
        }
    }
}
