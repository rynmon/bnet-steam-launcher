//! Drawing the window: the header, the game list, the options, the footer and the dialogs.

use eframe::egui::{self, Color32, RichText};

use super::{Action, App};
use crate::setup::{self, GameRow};

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

impl App {
    /// "Version X is available", with a button to install it when the release allows that.
    pub(super) fn update_notice(&mut self, ui: &mut egui::Ui) {
        let Some(release) = &self.newer else {
            return;
        };
        let (version, page, can_install) = (
            release.version.clone(),
            release.page.clone(),
            release.can_install(),
        );
        ui.horizontal(|ui| {
            ui.colored_label(
                Color32::from_rgb(90, 180, 90),
                format!("Version {version} is available."),
            );
            if !can_install {
                ui.hyperlink_to("Download it", &page);
                ui.weak("then open it and press Save to update your shortcuts.");
                return;
            }
            if self.updating.is_some() {
                ui.spinner();
                ui.label("Updating...");
                return;
            }
            if ui
                .add_enabled(self.working.is_none(), egui::Button::new("Update now"))
                .on_hover_text(
                    "Downloads it from GitHub, checks it against its published checksum, and \
                     reopens the app. Your shortcuts use the new version straight away.",
                )
                .clicked()
            {
                self.start_update();
            }
            ui.hyperlink_to("What's new", &page);
        });
    }

    pub(super) fn confirm_dialog(
        &mut self,
        ui: &mut egui::Ui,
        action: Action,
        steam_running: bool,
    ) {
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

    pub(super) fn header(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn game_list(&mut self, ui: &mut egui::Ui) {
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
            let ours = self.ours(&row).cloned();
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
            let note = match (ours.is_some(), want, row.installed) {
                (true, true, _) => Some(match (outdated, &ours) {
                    (Some("rename"), Some(s)) => {
                        format!("In Steam - will be renamed from \"{}\"", s.name)
                    }
                    (Some("settings"), _) => {
                        "In Steam - will be updated with the new settings".to_string()
                    }
                    (Some(_), _) => {
                        "In Steam - will be updated to this version of the app".to_string()
                    }
                    _ => "In Steam".to_string(),
                }),
                (true, false, _) => Some("Will be removed from Steam".into()),
                (false, true, false) => {
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

    pub(super) fn options(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn footer(&mut self, ui: &mut egui::Ui) {
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
