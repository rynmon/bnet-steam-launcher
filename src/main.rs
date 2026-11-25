// Release builds have no console window, so a Steam launch shows nothing on screen. Debug builds
// keep it, for development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod artwork;
mod bnet;
mod config;
mod gui;
mod net;
mod procs;
mod run;
mod setup;
mod steam;
mod time;
mod update;
mod vdf_bin;
mod vdf_text;
#[cfg(windows)]
mod winmsg;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

/// Name used for the executable, its log/lock files, and to recognise shortcuts it made.
pub const EXE_STEM: &str = "bnet-launcher";

/// Add Battle.net games to Steam, and launch them with Steam's in-game status, overlay and Steam
/// Input working. Run without arguments to open the setup window.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch a game through Battle.net (what the Steam shortcuts run).
    Run(run::RunArgs),
    /// List detected Steam accounts, Battle.net games, and shortcuts made by this app.
    List(AccountArgs),
    /// Add games to Steam, by launch code or name (e.g. WoWF).
    Add(EditArgs),
    /// Remove games this app added to Steam, by launch code or name.
    Remove(EditArgs),
    /// Download Battle.net's installer and run it (SteamOS: through a new Proton shortcut).
    InstallBnet(AccountArgs),
    /// Rewrite every shortcut this app made with the current version and settings.
    Update(SteamArgs),
    /// Remove this app's shortcuts, menu entry, settings and installed copy.
    Uninstall(SteamArgs),
}

#[derive(clap::Args)]
struct SteamArgs {
    #[command(flatten)]
    account: AccountArgs,
    /// Close Steam first if it's running, and start it again afterwards.
    #[arg(long)]
    restart_steam: bool,
}

#[derive(clap::Args)]
struct AccountArgs {
    /// Steam account ID (a folder name under Steam's userdata). Defaults to the last signed-in one.
    #[arg(long)]
    account: Option<u32>,
}

#[derive(clap::Args)]
struct EditArgs {
    /// Launch codes or game names.
    #[arg(required = true)]
    games: Vec<String>,
    #[command(flatten)]
    account: AccountArgs,
    /// Close Steam first if it's running, and start it again afterwards.
    #[arg(long)]
    restart_steam: bool,
    /// Edit this shortcuts.vdf instead of the account's (for testing; Steam may be running).
    #[arg(long, hide = true)]
    shortcuts_file: Option<PathBuf>,
    /// Point shortcuts at this launcher instead of installing a copy (for testing).
    #[arg(long, hide = true)]
    launcher: Option<PathBuf>,
    /// Also download artwork from SteamGridDB (with this key, or the one saved in the app).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    artwork: Option<String>,
}

fn main() {
    let cli = Cli::parse_from(std::env::args_os());
    #[cfg(windows)]
    if matches!(
        cli.command,
        Some(
            Command::List(_)
                | Command::Add(_)
                | Command::Remove(_)
                | Command::InstallBnet(_)
                | Command::Update(_)
                | Command::Uninstall(_)
        )
    ) {
        winmsg::attach_parent_console();
    }
    let code = match cli.command {
        None => report(gui::run()),
        Some(Command::Run(args)) => run::run(args),
        Some(Command::List(args)) => report(list(&args)),
        Some(Command::Add(args)) => report(edit(&args, true)),
        Some(Command::Remove(args)) => report(edit(&args, false)),
        Some(Command::InstallBnet(args)) => report(install_bnet(&args)),
        Some(Command::Update(args)) => report(update_all(&args)),
        Some(Command::Uninstall(args)) => report(uninstall(&args)),
    };
    std::process::exit(code);
}

fn report(result: Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e:#}");
            #[cfg(windows)]
            winmsg::error_box("Battle.net games for Steam", &format!("{e:#}"));
            1
        }
    }
}

fn pick_account(steam: &steam::SteamInstall, id: Option<u32>) -> Result<steam::Account> {
    let accounts = steam.accounts();
    match id {
        Some(id) => accounts
            .into_iter()
            .find(|a| a.id == id)
            .with_context(|| format!("no Steam account {id}")),
        None => accounts
            .into_iter()
            .next()
            .context("no Steam accounts found - sign in to Steam once first"),
    }
}

fn list(args: &AccountArgs) -> Result<()> {
    let steam = steam::SteamInstall::find().context("Steam wasn't found")?;
    println!("Steam: {}", steam.root.display());
    for a in steam.accounts() {
        println!(
            "  account {} ({}){}",
            a.id,
            a.name,
            if a.most_recent {
                ", last signed in"
            } else {
                ""
            }
        );
    }
    let envs = bnet::find_bnet();
    if envs.is_empty() {
        println!("Battle.net: not found");
    }
    for e in &envs {
        println!("Battle.net: {}", e.bnet_exe.display());
    }
    println!("Games:");
    for row in setup::game_rows(envs.first(), &config::Config::load()) {
        if row.installed {
            println!(
                "  {:<8} {} (installed: {})",
                row.code,
                row.name,
                row.game_dirs.join(", ")
            );
        }
    }
    let account = pick_account(&steam, args.account)?;
    println!("Shortcuts in account {}:", account.id);
    for s in steam::Shortcuts::load(&account.shortcuts_path())?.entries() {
        let kind = if s.is_ours() {
            "app"
        } else if s.is_legacy() {
            "old script"
        } else {
            continue;
        };
        println!(
            "  {} [{}] ({kind}): {}",
            s.name,
            s.launch_code().unwrap_or_default(),
            s.launch_options
        );
    }
    Ok(())
}

fn edit(args: &EditArgs, add: bool) -> Result<()> {
    let steam = steam::SteamInstall::find().context("Steam wasn't found")?;
    let envs = bnet::find_bnet();
    let rows = setup::game_rows(envs.first(), &config::Config::load());
    let mut picked = Vec::new();
    for g in &args.games {
        let row = rows
            .iter()
            .find(|r| r.code.eq_ignore_ascii_case(g) || r.name.eq_ignore_ascii_case(g))
            .with_context(|| {
                format!(
                    "unknown game \"{g}\" - use its launch code, e.g. WoWF (see LAUNCH_CODES.md)"
                )
            })?;
        picked.push(row);
    }
    let changes = if add {
        setup::Changes {
            add: picked,
            ..Default::default()
        }
    } else {
        setup::Changes {
            remove: picked,
            ..Default::default()
        }
    };

    let (path, real) = match &args.shortcuts_file {
        Some(p) => (p.clone(), false),
        None => (
            pick_account(&steam, args.account.account)?.shortcuts_path(),
            true,
        ),
    };
    let restart = real && args.restart_steam && steam::SteamInstall::is_running();
    if real && !restart && steam::SteamInstall::is_running() {
        bail!("Steam is running. Close it first, or pass --restart-steam.");
    }
    if restart {
        if setup::in_game_mode() {
            bail!("Switch to Desktop Mode first: Steam can't be restarted in Game Mode.");
        }
        println!("Closing Steam...");
        steam.shutdown()?;
    }
    let result = setup::apply(
        &steam,
        &path,
        envs.first(),
        &changes,
        args.launcher.as_deref(),
        real,
    )
    .and_then(|applied| {
        for line in &applied.lines {
            println!("{line}");
        }
        if let Some(key) = &args.artwork {
            let key = if key.is_empty() {
                config::Config::load().steamgriddb_key
            } else {
                key.clone()
            };
            if key.is_empty() {
                bail!(
                    "--artwork needs a SteamGridDB API key: --artwork <key>, or save one in the app"
                );
            }
            let grid = path
                .parent()
                .context("shortcuts file has no folder")?
                .join("grid");
            for (appid, name) in &applied.added {
                match artwork::fetch(name, *appid, &grid, &key) {
                    Ok(line) => println!("{line}"),
                    Err(e) => println!("{name}: artwork failed: {e:#}"),
                }
            }
        }
        Ok(())
    });
    if restart {
        println!("Starting Steam...");
        steam.start()?;
    }
    result
}

fn install_bnet(args: &AccountArgs) -> Result<()> {
    if cfg!(windows) {
        println!("{}", setup::install_bnet_windows()?);
        return Ok(());
    }
    let steam = steam::SteamInstall::find().context("Steam wasn't found")?;
    if setup::in_game_mode() {
        bail!("Switch to Desktop Mode first: Steam can't be restarted in Game Mode.");
    }
    let account = pick_account(&steam, args.account)?;
    println!("Closing Steam...");
    steam.shutdown()?;
    let result = setup::add_bnet_installer(&steam, &account.shortcuts_path());
    println!("Starting Steam...");
    match &result {
        Ok(appid) => steam.start_with(&[&format!(
            "steam://rungameid/{}",
            steam::shortcut_game_id(*appid)
        )])?,
        Err(_) => steam.start()?,
    }
    result?;
    println!(
        "Battle.net's installer will open through Steam. Install Battle.net, sign in and install your games, then run this app again to add them."
    );
    Ok(())
}

/// Runs `work` with Steam closed: closes and restarts it if `restart` (and it's running), else
/// refuses while it's running.
fn with_steam_closed<T>(
    steam: &steam::SteamInstall,
    restart: bool,
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let running = steam::SteamInstall::is_running();
    if running && !restart {
        bail!("Steam is running. Close it first, or pass --restart-steam.");
    }
    if running {
        if setup::in_game_mode() {
            bail!("Switch to Desktop Mode first: Steam can't be restarted in Game Mode.");
        }
        println!("Closing Steam...");
        steam.shutdown()?;
    }
    let result = work();
    if running {
        println!("Starting Steam...");
        steam.start()?;
    }
    result
}

fn update_all(args: &SteamArgs) -> Result<()> {
    let steam = steam::SteamInstall::find().context("Steam wasn't found")?;
    let account = pick_account(&steam, args.account.account)?;
    let envs = bnet::find_bnet();
    let rows = setup::game_rows(envs.first(), &config::Config::load());
    let shortcuts = steam::Shortcuts::load(&account.shortcuts_path())?.entries();
    let update: Vec<&setup::GameRow> = rows
        .iter()
        .filter(|r| {
            setup::shortcuts_for(r, &shortcuts)
                .iter()
                .any(|s| s.is_ours())
        })
        .collect();
    if update.is_empty() {
        println!("No shortcuts made by this app in account {}.", account.id);
        return Ok(());
    }
    let changes = setup::Changes {
        update,
        ..Default::default()
    };
    let applied = with_steam_closed(&steam, args.restart_steam, || {
        setup::apply(
            &steam,
            &account.shortcuts_path(),
            envs.first(),
            &changes,
            None,
            true,
        )
    })?;
    for line in applied.lines {
        println!("{line}");
    }
    Ok(())
}

fn uninstall(args: &SteamArgs) -> Result<()> {
    let steam = steam::SteamInstall::find().context("Steam wasn't found")?;
    let account = pick_account(&steam, args.account.account)?;
    let envs = bnet::find_bnet();
    let lines = with_steam_closed(&steam, args.restart_steam, || {
        setup::uninstall(&steam, &account.shortcuts_path(), envs.first())
    })?;
    for line in lines {
        println!("{line}");
    }
    Ok(())
}
