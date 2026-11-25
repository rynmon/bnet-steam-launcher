# bnet-steam-launcher

[![License](https://img.shields.io/github/license/rynmon/bnet-steam-launcher)](https://github.com/rynmon/bnet-steam-launcher/blob/main/LICENSE)
[![Downloads](https://img.shields.io/github/downloads/rynmon/bnet-steam-launcher/total)](https://github.com/rynmon/bnet-steam-launcher/releases)
![Windows](https://img.shields.io/badge/Windows-supported-0078D6?logo=windows&logoColor=white)
![Steam Deck](https://img.shields.io/badge/Steam%20Deck-supported-1b2838?logo=steamdeck&logoColor=white)
![Steam Machine](https://img.shields.io/badge/Steam%20Machine-supported-1b2838?logo=steam&logoColor=white)
[![Build](https://github.com/rynmon/bnet-steam-launcher/actions/workflows/app.yml/badge.svg)](https://github.com/rynmon/bnet-steam-launcher/actions/workflows/app.yml)

Add your Battle.net games to Steam and play them from there — with Steam's **in-game status**, **overlay** and **Steam Input** working, on **Windows** and **SteamOS** (Steam Deck and Steam Machine).

Open the app, tick your games, press **Save to Steam**. Each game gets its own Steam shortcut; pressing Play starts Battle.net, launches the game, and closes Battle.net again when you quit.

Tested with **World of Warcraft: Forever** on Windows and on a Steam Deck. Other Battle.net games use the same mechanism — see [Launch codes](LAUNCH_CODES.md).

## Why not just add Battle.net to Steam?

Launching `Battle.net.exe --exec="launch WoWF"` directly from Steam breaks in a few ways:

- Steam drops the in-game status within seconds, because that command hands the launch to Battle.net and returns immediately.
- The overlay and Steam Input only attach to programs started by the Steam launch, so a Battle.net that's already running never gets them.
- On SteamOS, Battle.net only runs under Proton, where naively resending the launch command can hang.

This app instead starts Battle.net from the Steam launch itself, waits until it's ready, sends the launch command, waits for the actual game, and stays running until the game exits — then closes Battle.net so Steam's status clears.

## Setup

### Windows

1. Download `bnet-launcher-windows-x86_64.exe` from [Releases](https://github.com/rynmon/bnet-steam-launcher/releases/latest) and open it.

   The app isn't code-signed, so Windows may say *"Windows protected your PC"* the first time. Click **More info → Run anyway**. Some antivirus tools may also be wary of it, since it closes Battle.net and edits Steam's files.
2. If Battle.net isn't installed, press **Install Battle.net**: it downloads and opens Battle.net's official installer. Install Battle.net, sign in, install your games, then press **Refresh**.
3. Tick the games you want and press **Save to Steam**. Steam has to be closed while its shortcuts change, so the app offers to close it and starts it again afterwards.
4. Optional: set a controller layout in each new shortcut's Properties.

After the first run the app is in the Start menu as **bnet-steam-launcher**, so you can delete the download.

### SteamOS (Steam Deck, Steam Machine)

1. Switch to **Desktop Mode** and download `bnet-launcher-linux-x86_64` from [Releases](https://github.com/rynmon/bnet-steam-launcher/releases/latest). Right-click it → Properties → Permissions → tick **Is executable**, then open it.
2. If Battle.net isn't installed, press **Install Battle.net**. It downloads Battle.net's official installer and adds a **"Battle.net"** shortcut to Steam that runs it with Proton (GE-Proton if you have it installed), then starts it. Install Battle.net with the default path, sign in, install your games, then press **Refresh**.

   **Keep that "Battle.net" shortcut.** Battle.net and your games are installed inside its Proton prefix; removing the shortcut can make Steam delete them. The app never removes it.
3. Tick the games you want. Open **Options** and tick **Close Battle.net when a game starts**, so Battle.net's window doesn't keep the controller while you play.
4. Press **Save to Steam**. The app closes Steam, adds the shortcuts and starts Steam again.
5. Optional: set a controller layout in each new shortcut's Properties.
6. Go back to Game Mode and press Play.

After the first run the app is in the application menu as **bnet-steam-launcher**.

## Using the app

- **Games:** installed games are listed and ticked if they're already in Steam. Tick to add, untick to remove — only shortcuts the app made are ever removed. Installed something new? Press **Refresh**. **Show games that aren't installed or aren't in the game list** reveals the rest, including installed games the app doesn't know (marked *launch code guessed*).
- **Advanced** (under each ticked game): how long to wait for the game to start, how often to resend the launch command, how long to let Battle.net settle first, and how long to keep watching after the game exits. Leave them at their defaults unless a game needs it.
- **Options:**
  - **Close Battle.net when a game starts** — Battle.net's own "When I launch a game → Exit Battle.net completely" setting. Recommended on SteamOS. Battle.net has to be closed to change it.
  - **Artwork from SteamGridDB** — paste a free [SteamGridDB API key](https://www.steamgriddb.com/profile/preferences/api) and the app downloads a cover, wide cover, background and logo for each shortcut it adds. The artwork is made by the SteamGridDB community; anything you've already set is never replaced.
  - **Check for new versions** — the window shows a link when a newer release is out. It only asks GitHub for the latest release; nothing about you is sent.
  - **Open launch log** and **Copy details for a bug report** — for when something goes wrong.
  - **Uninstall…** — see below.
### Updating

When a new version is out, the window says so. Press **Update now**: the app downloads it from GitHub, checks it against the release's published SHA-256 checksums (`SHA256SUMS`), replaces the installed copy the shortcuts run, and reopens. If the footer then shows **Update N**, press **Save to Steam** so those shortcuts get the new version's launch options.

You can also update by hand: download the new version, open it, and press **Save to Steam**.

### Uninstalling

Open **Options → Uninstall…**. It removes the shortcuts the app made, their Proton settings, the Start menu / application menu entry, the app's settings and its installed copy. Battle.net, your games, and shortcuts you made yourself are left alone (on SteamOS, so is the "Battle.net" shortcut holding the prefix).

## How it works

Each shortcut runs the app's installed copy (`%LOCALAPPDATA%\bnet-launcher\bnet-launcher.exe` on Windows, `~/.local/share/bnet-launcher/bnet-launcher` on SteamOS) like this:

```
bnet-launcher run --code WoWF --process Wow --process WowB --game-dir "O:/Blizzard/World of Warcraft"
```

On SteamOS the shortcut's target is `Battle.net.exe` in the Proton prefix, and the launch options wrap Steam's own command: `STEAM_COMPAT_DATA_PATH=<prefix> <bnet-launcher> run ... -- %command%`. Each game shortcut shares the prefix Battle.net is installed in and uses the same Proton version.

When you press Play, the launcher:

1. Checks whether the game is already running. If it is, it leaves Battle.net alone and just keeps Steam's status up until the game exits.
2. Otherwise closes any running Battle.net and starts it again from the Steam launch, so the game becomes part of it.
3. Waits for Battle.net, then sends the launch command, resending it every few seconds until the game appears.
4. Recognises the game by its process name **or** by anything started from its install folder (read from Battle.net's own records), so a renamed game exe can't cause repeated launches.
5. Waits until the game exits. If it comes back within a few seconds (a self-restart after a patch), it keeps waiting.
6. Closes Battle.net, which clears Steam's in-game status. On SteamOS, if anything is left over, it ends the whole Wine session for that prefix.

If the game doesn't start, an error box (Windows) or notification (SteamOS Desktop Mode) explains the likely cause — usually a pending game update in Battle.net.

You can edit the options in a shortcut's Launch Options:

| Option | Default | Description |
|---|---|---|
| `--code` | required | Battle.net launch code, e.g. `WoWF`. See [Launch codes](LAUNCH_CODES.md). |
| `--process` | | Game process name, with or without `.exe`. Repeatable. |
| `--game-dir` | | Game install folder; anything started from it counts as the game. Repeatable. At least one of `--process` / `--game-dir` is needed. |
| `--bnet-exe` | auto-detect | Windows: path to `Battle.net.exe`. |
| `--bnet-max-wait` | `20` (SteamOS `30`) | Seconds to wait for Battle.net before sending the launch code anyway. |
| `--settle-delay` | `3` (SteamOS `8`) | Seconds to wait after Battle.net appears. Raise it if the first launch command is ignored. |
| `--retry-after` | `10` | Resend the launch code every this-many seconds until the game appears. |
| `--startup-wait` | `120` | Give up if the game hasn't started after this many seconds. |
| `--relaunch-grace` | `10` | Seconds to keep watching after the game exits, in case it restarts itself. `0` disables. |
| `--no-popup` | off | Don't show the error box / notification when a launch fails. |

Saving in the app rewrites the launch options, so prefer the **Advanced** settings there for the timings.

### Safety

- Steam's shortcut file is backed up before every change (`shortcuts.vdf.bnet-launcher-<time>.bak` next to it, last five kept), and is only written while Steam is closed.
- On SteamOS, Steam's settings file (`config.vdf`) is only changed if the app can read it and write it back exactly as it was; otherwise it's left alone and you're asked to set the shortcut's Proton version yourself.
- Battle.net's settings file is backed up before the "close when a game starts" setting is changed.
- A lock file stops two launches (Play pressed twice) from interfering with each other.

## Command line

The same things work from a terminal (on Windows, from a console):

```
bnet-launcher list                 # what it finds: Steam, accounts, games, its shortcuts
bnet-launcher add WoWF             # add games by launch code or name
bnet-launcher remove WoWF
bnet-launcher update               # refresh every shortcut it made
bnet-launcher install-bnet
bnet-launcher uninstall
```

Add `--restart-steam` to let it close and restart Steam, and `--artwork` (to `add`) to download artwork with the saved SteamGridDB key.

## Known limits

- **WoW Retail and WoW Forever share an install folder.** If one is already running when you press Play on the other, the launcher treats the running one as the game and tracks it instead of starting the other.
- **Games other than World of Warcraft: Forever are untested.** Their launch codes come from community sources, and process names for some are best guesses — the install-folder check usually covers a wrong name.
- **Battle.net is closed when you start and quit a game**, which interrupts downloads in progress and means it isn't open for chat unless you open it yourself.
- **Changing shortcuts needs Steam closed**, so on SteamOS it has to be done from Desktop Mode.

## Troubleshooting

| Symptom | Fix |
|---|---|
| "Windows protected your PC" when opening the app | It isn't code-signed. Click **More info → Run anyway**. |
| "The game didn't start" | Usually Battle.net is downloading an update for the game: open Battle.net, let it finish, press Play in Steam again. If it keeps happening, raise the startup wait in the game's **Advanced** settings. |
| Battle.net opens but the game doesn't start | Raise the settle delay in the game's **Advanced** settings (try 10–20 on SteamOS). |
| "Steam didn't exit within a minute" | Close Steam yourself (Steam → Exit), then save again. |
| "Battle.net's setting wasn't changed" | Battle.net was open, and it overwrites its settings when it closes. Close Battle.net, then save again. |
| A game is missing from the list | Install it in Battle.net, then press **Refresh**. Still missing? Tick "Show games that aren't installed or aren't in the game list". |
| "launch code guessed" next to a game | It isn't in the app's game list, so Battle.net's product code is used as the launch code. If it doesn't start, look for its code in [Launch codes](LAUNCH_CODES.md) and open an issue. |
| "artwork failed" | Check the SteamGridDB API key in Options. You can also set artwork in Steam (right-click the game → Manage). |
| Controller input also drives Battle.net's window while playing (SteamOS) | Tick **Close Battle.net when a game starts** in Options. |
| Steam Input doesn't work | Set a gamepad layout in the shortcut's Properties → Controller. |
| Steam still shows "In-game" ~10 seconds after quitting | Expected: the launcher watches briefly in case the game restarts itself. Lower the relaunch grace in **Advanced**, or set it to 0. |
| (SteamOS) "isn't in the layout this app can safely rewrite" | Steam's `config.vdf` was deliberately left alone. Set each new shortcut's Proton version yourself: Properties → Compatibility. |
| (SteamOS) "Not found in any Proton prefix" | Press **Install Battle.net**, or make sure your Battle.net is inside a Steam Proton prefix (`steamapps/compatdata/<id>/pfx`). |
| (SteamOS) Removing a shortcut wiped the install | That was the shortcut holding the Proton prefix — reinstall with **Install Battle.net**. The app itself never removes that shortcut. |

Still stuck? Use **Options → Copy details for a bug report** and paste it into a [new issue](https://github.com/rynmon/bnet-steam-launcher/issues). The launch log is at `%TEMP%\bnet-launcher.log` (Windows) or `/run/user/1000/bnet-launcher.log` (SteamOS); the previous launch's is `bnet-launcher.prev.log`.

## Building from source

Install [Rust](https://rustup.rs) (on Windows also the Visual Studio Build Tools' C++ workload; on Linux the `libxkbcommon` and `libxcb` development packages), then:

```
cargo build --release
cargo test
```

The game list is [games.json](games.json); a test keeps it in step with [LAUNCH_CODES.md](LAUNCH_CODES.md). The app icon is drawn by [assets/make_icon.py](assets/make_icon.py). Pushing a `v*` tag builds both executables and attaches them to the GitHub release.

## Disclaimer

Not affiliated with Blizzard, Battle.net, or Valve. The launcher force-closes Battle.net and, on SteamOS, can end a Wine session — use at your own risk.

## License

MIT, see [LICENSE](LICENSE).
