//! `bnet-launcher run`: what a Steam shortcut runs when you press Play.
//!
//! Steam only shows "In-game" while the process it launched is alive, and the overlay/Steam Input
//! only attach to processes descended from that launch. Sending `--exec="launch <code>"` to an
//! already-running Battle.net returns immediately and the game starts outside Steam's launch, so
//! both break. Instead this (re)starts Battle.net itself, so the game becomes a descendant of the
//! Steam launch, sends the launch command, waits for the game, and stays alive until it exits.
//! Battle.net is then closed so Steam clears the in-game status.
//!
//! On Linux the shortcut's target is Battle.net.exe under Proton, and the launch options end in
//! `-- %command%`: the Proton command Steam built is run from here, and the launch command is sent
//! with a direct `proton run` call (re-running Steam's whole container chain can start a second,
//! separate Wine session that never reaches the running Battle.net).

use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::procs::{self, Proc};

#[derive(clap::Args, Debug, Clone)]
pub struct RunArgs {
    /// Battle.net launch code, e.g. WoWF.
    #[arg(long)]
    pub code: String,
    /// Game process name (with or without .exe). Repeatable.
    #[arg(long = "process")]
    pub processes: Vec<String>,
    /// Game install folder (Windows path, as Battle.net stores it). Any program started from it
    /// counts as the game, so a wrong or changed process name can't cause repeated launches.
    /// Repeatable.
    #[arg(long = "game-dir")]
    pub game_dirs: Vec<String>,
    /// Windows: full path to Battle.net.exe (auto-detected when omitted).
    #[arg(long)]
    pub bnet_exe: Option<PathBuf>,
    /// Max seconds to wait for Battle.net before sending the launch command anyway [Windows 20, Linux 30].
    #[arg(long)]
    pub bnet_max_wait: Option<u64>,
    /// Extra seconds to wait after Battle.net appears [Windows 3, Linux 8].
    #[arg(long)]
    pub settle_delay: Option<u64>,
    /// Resend the launch command every this-many seconds until the game appears.
    #[arg(long, default_value_t = 10)]
    pub retry_after: u64,
    /// Give up if the game hasn't appeared after this many seconds.
    #[arg(long, default_value_t = 120)]
    pub startup_wait: u64,
    /// After the game exits, watch this many seconds for it to come back (self-restart after a
    /// patch). 0 disables.
    #[arg(long, default_value_t = 10)]
    pub relaunch_grace: u64,
    /// Don't show an error popup/notification when the launch fails.
    #[arg(long)]
    pub no_popup: bool,
    /// Linux: the command Steam built for this shortcut (`-- %command%`).
    #[arg(last = true)]
    pub command: Vec<String>,
}

// --- Logging ----------------------------------------------------------------------------------

static LOG: Mutex<Option<File>> = Mutex::new(None);

pub fn log(msg: &str) {
    let line = format!("[{}] {msg}\n", crate::time::hms());
    if let Ok(mut f) = LOG.lock()
        && let Some(f) = f.as_mut()
    {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
    eprint!("{line}");
}

fn run_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir()
    } else {
        std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
    }
}

pub fn log_path() -> PathBuf {
    run_dir().join(format!("{}.log", crate::EXE_STEM))
}

// --- Cleanup ----------------------------------------------------------------------------------

/// What has to be undone on exit. Kept global so a Stop from Steam (SIGTERM on Linux) can run it.
struct Cleanup {
    lock: PathBuf,
    started_bnet: bool,
    wineprefix: Option<PathBuf>,
    wineserver: Option<PathBuf>,
}

static CLEANUP: Mutex<Option<Cleanup>> = Mutex::new(None);

fn set_started_bnet() {
    if let Ok(mut c) = CLEANUP.lock()
        && let Some(c) = c.as_mut()
    {
        c.started_bnet = true;
    }
}

/// Runs once; later calls do nothing.
fn cleanup() {
    let Some(c) = CLEANUP.lock().ok().and_then(|mut c| c.take()) else {
        return;
    };
    // Battle.net is only closed if this run started it: when the game was already running, the
    // Battle.net session belongs to whoever started it.
    if c.started_bnet {
        stop_bnet(c.wineprefix.as_deref(), c.wineserver.as_deref());
    }
    let _ = std::fs::remove_file(&c.lock);
}

// --- Finding the game -------------------------------------------------------------------------

struct GameMatcher {
    names: Vec<String>,
    dirs: Vec<String>,
}

/// Programs in a game's folder that aren't the game.
const NOT_THE_GAME: &[&str] = &[
    "launcher",
    "blizzarderror",
    "crash",
    "werfault",
    "battle.net",
    "uninstall",
    "setup",
];

impl GameMatcher {
    fn new(args: &RunArgs) -> Self {
        GameMatcher {
            names: args.processes.iter().map(|p| procs::exe_name(p)).collect(),
            dirs: args
                .game_dirs
                .iter()
                .map(|d| {
                    let d = procs::normalize(d.trim_end_matches(['/', '\\']));
                    format!("{d}/")
                })
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.dirs.is_empty()
    }

    fn is_game(&self, p: &Proc) -> bool {
        if self.names.contains(&p.name) {
            return true;
        }
        p.path.as_ref().is_some_and(|path| {
            self.dirs.iter().any(|d| path.starts_with(d))
                && !NOT_THE_GAME.iter().any(|n| p.name.contains(n))
        })
    }

    fn find(&self) -> Option<Proc> {
        procs::snapshot().into_iter().find(|p| self.is_game(p))
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.names.is_empty() {
            parts.push(self.names.join(" / "));
        }
        if !self.dirs.is_empty() {
            parts.push(format!("anything in {}", self.dirs.join(" or ")));
        }
        parts.join(", or ")
    }
}

fn bnet_pids() -> Vec<u32> {
    procs::snapshot()
        .into_iter()
        .filter(|p| p.name == "battle.net.exe")
        .map(|p| p.pid)
        .collect()
}

/// Closes every Battle.net process (Linux: politely first, then by force, then the whole Wine
/// session as a last resort, which guarantees Steam's tracked launch actually ends).
fn stop_bnet(wineprefix: Option<&std::path::Path>, wineserver: Option<&std::path::Path>) {
    let pids = bnet_pids();
    if !pids.is_empty() {
        procs::stop(&pids, false);
    }
    let deadline = Instant::now() + Duration::from_secs(if cfg!(windows) { 5 } else { 2 });
    while !bnet_pids().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
    }
    let left = bnet_pids();
    if !left.is_empty() {
        procs::stop(&left, true);
    }
    if let (Some(prefix), Some(ws)) = (wineprefix, wineserver)
        && prefix.is_dir()
    {
        let _ = std::process::Command::new(ws)
            .arg("-k")
            .env("WINEPREFIX", prefix)
            .status();
    }
}

// --- Platform pieces --------------------------------------------------------------------------

/// How Battle.net is started and given the launch command on this platform.
struct Launch {
    /// Windows: Battle.net.exe. Linux: the target exe path from Steam's command.
    bnet_exe: PathBuf,
    /// Linux: Steam's full command, and the Proton binary pulled out of it.
    command: Vec<String>,
    proton: Option<PathBuf>,
}

impl Launch {
    #[cfg(windows)]
    fn new(args: &RunArgs) -> anyhow::Result<Self> {
        let bnet_exe = match &args.bnet_exe {
            Some(p) if p.is_file() => p.clone(),
            Some(p) => anyhow::bail!(
                "Battle.net not found at the path given in --bnet-exe: {}",
                p.display()
            ),
            None => crate::bnet::find_bnet()
                .into_iter()
                .next()
                .map(|e| e.bnet_exe)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Could not find Battle.net.exe. Pass its full path with --bnet-exe."
                    )
                })?,
        };
        log(&format!("Battle.net: {}", bnet_exe.display()));
        Ok(Launch {
            bnet_exe,
            command: Vec::new(),
            proton: None,
        })
    }

    #[cfg(not(windows))]
    fn new(args: &RunArgs) -> anyhow::Result<Self> {
        if args.command.is_empty() {
            anyhow::bail!("No command received - Launch Options must end with -- %command%");
        }
        log(&format!("Base command: {}", args.command.join(" ")));
        let bnet_exe = PathBuf::from(args.command.last().expect("not empty"));
        let proton = args
            .command
            .iter()
            .find(|a| a.ends_with("/proton"))
            .map(PathBuf::from)
            .filter(|p| p.is_file());
        match &proton {
            Some(p) => {
                log(&format!("Proton binary: {}", p.display()));
                log(&format!("Target exe: {}", bnet_exe.display()));
            }
            None => log(
                "Could not identify the Proton binary from the base command - will fall back to resending the full command",
            ),
        }
        Ok(Launch {
            bnet_exe,
            command: args.command.clone(),
            proton,
        })
    }

    /// Linux: the Wine prefix root is everything before "/drive_c/" in the target exe path.
    fn wineprefix(&self) -> Option<PathBuf> {
        let s = self.bnet_exe.to_string_lossy();
        s.find("/drive_c/").map(|i| PathBuf::from(&s[..i]))
    }

    /// Linux: Proton's own wineserver (files/bin on current builds, dist/bin on old ones), else PATH.
    fn wineserver(&self) -> Option<PathBuf> {
        if cfg!(windows) {
            return None;
        }
        let from_proton = self
            .proton
            .as_ref()
            .and_then(|p| p.parent())
            .and_then(|dir| {
                ["files/bin/wineserver", "dist/bin/wineserver"]
                    .iter()
                    .map(|c| dir.join(c))
                    .find(|c| c.is_file())
            });
        from_proton.or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|d| d.join("wineserver"))
                    .find(|c| c.is_file())
            })
        })
    }

    fn start_bnet(&self) -> anyhow::Result<Option<std::process::Child>> {
        let mut cmd = if cfg!(windows) {
            std::process::Command::new(&self.bnet_exe)
        } else {
            let mut c = std::process::Command::new(&self.command[0]);
            c.args(&self.command[1..]);
            c
        };
        Ok(Some(cmd.spawn()?))
    }

    fn send_launch(&self, code: &str) {
        let mut cmd;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd = std::process::Command::new(&self.bnet_exe);
            // Exactly the form Battle.net documents: --exec="launch <code>".
            cmd.raw_arg(format!("--exec=\"launch {code}\""));
        }
        #[cfg(not(windows))]
        {
            match &self.proton {
                Some(proton) => {
                    cmd = std::process::Command::new(proton);
                    cmd.arg("run").arg(&self.bnet_exe);
                }
                None => {
                    cmd = std::process::Command::new(&self.command[0]);
                    cmd.args(&self.command[1..]);
                }
            }
            cmd.arg(format!("--exec=launch {code}"));
        }
        match cmd.spawn() {
            // Reaped in the background so it never blocks the wait loop.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => log(&format!("Could not send the launch command: {e}")),
        }
    }

    fn bnet_ready(&self) -> bool {
        #[cfg(windows)]
        {
            procs::has_window(&bnet_pids())
        }
        #[cfg(not(windows))]
        {
            !bnet_pids().is_empty()
        }
    }
}

// --- The run itself ---------------------------------------------------------------------------

pub fn run(args: RunArgs) -> i32 {
    let dir = run_dir();
    let log_file = log_path();
    let lock = dir.join(format!("{}.lock", crate::EXE_STEM));

    // Single-instance guard: if another run is going (Play pressed twice), exit before touching
    // the log so the second run can't rotate away the one the first is still writing. The PID
    // must still be this app: a lock left by a hard-killed run can name a reused PID.
    if let Ok(text) = std::fs::read_to_string(&lock)
        && let Ok(pid) = text.trim().parse::<u32>()
        && pid != std::process::id()
        && procs::pid_mentions(pid, crate::EXE_STEM)
    {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_file)
        {
            let _ = writeln!(
                f,
                "[{}] Another instance (PID {pid}) is already running - exiting",
                crate::time::hms()
            );
        }
        return 1;
    }

    // Keep the previous run's log, so pressing Play again after a failure doesn't erase why.
    let _ = std::fs::rename(&log_file, dir.join(format!("{}.prev.log", crate::EXE_STEM)));
    if let Ok(f) = File::create(&log_file) {
        *LOG.lock().unwrap() = Some(f);
    }
    log(&format!(
        "=== {} {} run: {} ===",
        crate::EXE_STEM,
        env!("CARGO_PKG_VERSION"),
        std::env::args().collect::<Vec<_>>().join(" ")
    ));

    let _ = std::fs::write(&lock, std::process::id().to_string());
    *CLEANUP.lock().unwrap() = Some(Cleanup {
        lock,
        started_bnet: false,
        wineprefix: None,
        wineserver: None,
    });

    // Steam's Stop button (SIGTERM on Linux) or a closed terminal: clean up, then exit.
    let _ = ctrlc::set_handler(|| {
        log("Stopped - cleaning up");
        cleanup();
        std::process::exit(143);
    });

    let (code, failure) = match launch_and_wait(&args) {
        Ok(()) => (0, None),
        Err(Failure::NeverAppeared(msg)) => (2, Some(msg)),
        Err(Failure::Error(e)) => (1, Some(format!("ERROR: {e:#}"))),
    };
    if let Some(msg) = &failure {
        log(msg);
    }
    cleanup();
    // After cleanup, so Battle.net is already closed while the message is up.
    if let Some(msg) = failure
        && !args.no_popup
    {
        show_error(&msg);
    }
    log(&format!("Exit code {code}"));
    code
}

enum Failure {
    NeverAppeared(String),
    Error(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::Error(e)
    }
}

fn launch_and_wait(args: &RunArgs) -> Result<(), Failure> {
    let matcher = GameMatcher::new(args);
    if matcher.is_empty() {
        return Err(anyhow::anyhow!(
            "Give the game's --process name or --game-dir, e.g. --code WoWF --process Wow"
        )
        .into());
    }
    log(&format!(
        "Launch code: {} | Game: {}",
        args.code,
        matcher.describe()
    ));
    let launch = Launch::new(args)?;
    if let Ok(mut c) = CLEANUP.lock()
        && let Some(c) = c.as_mut()
    {
        c.wineprefix = launch.wineprefix();
        c.wineserver = launch.wineserver();
        if !cfg!(windows) {
            match &c.wineserver {
                Some(ws) => log(&format!("wineserver: {}", ws.display())),
                None => log(
                    "No wineserver found - the last-resort Wine session teardown on exit will be skipped",
                ),
            }
        }
    }

    let (max_wait, settle) = if cfg!(windows) { (20, 3) } else { (30, 8) };
    let max_wait = args.bnet_max_wait.unwrap_or(max_wait);
    let settle = args.settle_delay.unwrap_or(settle);

    let mut bnet_child = None;
    // 0. If the game is already running (started by hand, or Play pressed again), leave
    //    Battle.net alone and just track the running game.
    let mut game = matcher.find();
    if let Some(g) = &game {
        log(&format!(
            "Game already running ({} PID {}) - skipping Battle.net restart",
            g.name, g.pid
        ));
    } else {
        // 1. Restart Battle.net from here so the game becomes a descendant of the Steam launch.
        stop_bnet(
            launch.wineprefix().as_deref(),
            launch.wineserver().as_deref(),
        );
        if !cfg!(windows) {
            std::thread::sleep(Duration::from_secs(2));
        }
        set_started_bnet();
        bnet_child = launch
            .start_bnet()
            .map_err(|e| anyhow::anyhow!("Could not start Battle.net: {e}"))?;

        // 2. Wait for Battle.net to be up (up to max_wait), then let it settle.
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(max_wait) && !launch.bnet_ready() {
            std::thread::sleep(Duration::from_millis(500));
        }
        if launch.bnet_ready() {
            log(&format!(
                "Battle.net up after {:.1}s",
                started.elapsed().as_secs_f32()
            ));
        } else {
            log(&format!(
                "Battle.net not seen after {max_wait}s, continuing anyway"
            ));
        }
        std::thread::sleep(Duration::from_secs(settle));

        // 3. Send the launch command, and resend it every retry_after seconds until the game
        //    appears, so a silently ignored first attempt costs one interval, not the timeout.
        log("Sending launch command");
        launch.send_launch(&args.code);
        let deadline = Instant::now() + Duration::from_secs(args.startup_wait);
        let mut next_retry = Instant::now() + Duration::from_secs(args.retry_after.max(1));
        while game.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(1));
            game = matcher.find();
            if game.is_none() && Instant::now() >= next_retry {
                log("Game not seen yet, resending launch command");
                launch.send_launch(&args.code);
                next_retry = Instant::now() + Duration::from_secs(args.retry_after.max(1));
            }
        }
    }

    let Some(mut g) = game else {
        // A pending patch is by far the most common cause: Battle.net shows "Update" instead of
        // "Play" and quietly ignores the launch command, so say that first.
        return Err(Failure::NeverAppeared(format!(
            "The game didn't start within {}s.\n\n\
             Most likely Battle.net is downloading an update for it: open Battle.net, let the \
             update finish, then press Play in Steam again.\n\n\
             If there's no update, the launch code ({}) may be wrong for this game, or the game \
             may need longer to start (raise --startup-wait in the shortcut's Launch Options).",
            args.startup_wait, args.code
        )));
    };

    // 4. Stay alive until the game exits so Steam keeps the in-game status. If it comes back
    //    within relaunch_grace seconds (self-restart after a patch, launcher stub handoff), keep
    //    waiting on the new process instead of closing Battle.net under it.
    loop {
        log(&format!(
            "Game running ({} PID {}), waiting for exit",
            g.name, g.pid
        ));
        procs::wait_exit(g.pid);
        log("Game exited");
        let deadline = Instant::now() + Duration::from_secs(args.relaunch_grace);
        let mut back = None;
        while back.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            back = matcher.find();
        }
        match back {
            Some(b) => {
                log(&format!("Game came back as {} PID {}", b.name, b.pid));
                g = b;
            }
            None => break,
        }
    }

    // Linux: once Battle.net is closed, Steam's launch chain ends; wait for it briefly so Steam
    // sees a clean exit.
    cleanup();
    if let Some(mut child) = bnet_child {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}

// --- Telling the user about a failure ---------------------------------------------------------

/// Windows: an error box that closes itself after 30 seconds, so one hidden behind Big Picture
/// can't leave Steam stuck on "In-game". Linux: a desktop notification (Desktop Mode only).
fn show_error(msg: &str) {
    let text = format!("{msg}\n\nLog: {}", log_path().display());
    #[cfg(windows)]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            crate::winmsg::error_box("Battle.net launcher", &text);
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(Duration::from_secs(30));
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("notify-send")
            .args([
                "--app-name=Battle.net launcher",
                "--urgency=critical",
                "Battle.net launch failed",
                &text,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(processes: &[&str], dirs: &[&str]) -> RunArgs {
        RunArgs {
            code: "WoWF".into(),
            processes: processes.iter().map(|s| s.to_string()).collect(),
            game_dirs: dirs.iter().map(|s| s.to_string()).collect(),
            bnet_exe: None,
            bnet_max_wait: None,
            settle_delay: None,
            retry_after: 10,
            startup_wait: 120,
            relaunch_grace: 10,
            no_popup: true,
            command: Vec::new(),
        }
    }

    fn proc(name: &str, path: Option<&str>) -> Proc {
        Proc {
            pid: 1,
            name: name.into(),
            path: path.map(procs::normalize),
        }
    }

    #[test]
    fn wrong_process_name_still_found_by_folder() {
        // The bug that caused repeated launches: watching for WowB while the game runs as Wow.
        let m = GameMatcher::new(&args(&["WowB"], &["O:/Blizzard/World of Warcraft"]));
        assert!(m.is_game(&proc(
            "wow.exe",
            Some("O:\\Blizzard\\World of Warcraft\\_forever_\\Wow.exe")
        )));
        assert!(m.is_game(&proc("wowb.exe", None)));
    }

    #[test]
    fn folder_helpers_and_lookalikes_ignored() {
        let m = GameMatcher::new(&args(&["Wow"], &["O:/Blizzard/World of Warcraft/"]));
        assert!(!m.is_game(&proc(
            "world of warcraft launcher.exe",
            Some("o:/blizzard/world of warcraft/world of warcraft launcher.exe")
        )));
        assert!(!m.is_game(&proc(
            "blizzarderror.exe",
            Some("o:/blizzard/world of warcraft/_forever_/blizzarderror.exe")
        )));
        assert!(!m.is_game(&proc("wowclassic.exe", Some("c:/games/wowclassic.exe"))));
        // A sibling folder that merely starts with the same name.
        assert!(!m.is_game(&proc(
            "x.exe",
            Some("o:/blizzard/world of warcraft 2/x.exe")
        )));
    }
}
