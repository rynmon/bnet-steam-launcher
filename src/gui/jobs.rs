//! The work behind the window's buttons. Each runs on a background thread, so the window stays
//! responsive while Steam restarts.

use crate::bnet::BnetEnv;
use crate::setup;
use crate::steam::{Account, SteamInstall};

use super::Pending;

pub(super) struct SaveJob<'a> {
    pub steam: &'a SteamInstall,
    pub account: Option<Account>,
    pub env: Option<BnetEnv>,
    pub pending: &'a Pending,
    /// Battle.net's "exit when a game starts" setting, if it should change.
    pub setting: Option<bool>,
    /// SteamGridDB key; empty = no artwork.
    pub artwork_key: &'a str,
    pub restart_steam: bool,
}

pub(super) fn save(job: &SaveJob) -> anyhow::Result<Vec<String>> {
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

pub(super) fn install_bnet(
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

pub(super) fn uninstall(
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
