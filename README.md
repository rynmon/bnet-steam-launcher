# bnet-steam-launcher

![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20SteamOS-lightgrey.svg)
![Shell](https://img.shields.io/badge/shell-PowerShell%20%7C%20Bash-89e051.svg)
![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)

Launch a Battle.net game (built and tested with **WoW Forever**) from Steam with working **in-game status**, **overlay**, and **Steam Input**. Status clears automatically when you exit the game.

Works on **Windows** (PowerShell) and **SteamOS** — Steam Deck and Steam Machine (Bash + Proton). Tested on all three.

## Why not just add Battle.net as a shortcut?

Launching `Battle.net.exe --exec="launch WoWF"` directly from Steam breaks in a few ways: Steam drops the in-game status within seconds because the launch command hands off and returns immediately; the overlay and Steam Input only attach to processes descended from the Steam launch, so an already-running Battle.net never gets them; and on SteamOS, Battle.net has no native client and has to run under Proton, where naively resending the launch command can hang.

This script instead restarts Battle.net from the Steam launch itself, waits for it to be ready, sends the launch command, waits for the actual game process, and stays alive (keeping the status up) until the game exits — then closes Battle.net so the status clears.

Two versions, same name: `start-bnet-game.ps1` (Windows) and `start-bnet-game.sh` (SteamOS/Linux).

## Requirements

- **Windows:** PowerShell 5.1+ (built in), Steam, Battle.net desktop app.
- **SteamOS:** Steam, the Windows Battle.net installer run once under Proton (GE-Proton recommended) — no native Linux client exists.
- Tested with WoW Forever only. Other games use the same mechanism and should work — see [Launch codes](#launch-codes) below.

---

## Setup — Windows

1. Save `start-bnet-game.ps1`, e.g. to `C:\Scripts\start-bnet-game.ps1`.
2. Steam → **Add a Non-Steam Game → Browse** (All Files), add:
   ```
   C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
   ```
   (Don't add the `.ps1` directly — Steam can't run it.)
3. Right-click the entry → **Properties** → rename it, then set **Launch Options**:
   ```
   -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "C:\Scripts\start-bnet-game.ps1" -LaunchCode WoWF -GameProcess WowB
   ```
   **The `-File` path must be quoted if it contains any spaces** — e.g. anything under `Documents`, `Program Files`, or `OneDrive`:
   ```
   -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "C:\Users\you\Documents\Steam Tools\start-bnet-game.ps1" -LaunchCode WoWF -GameProcess WowB
   ```
   Without quotes, Windows splits the path at the space and tries to run something that doesn't exist.
4. Set a controller layout in Properties if you use one.

| Parameter | Default | Description |
|---|---|---|
| `-LaunchCode` | required | Battle.net's `--exec="launch <code>"` code, e.g. `WoWF`. |
| `-GameProcess` | required | Game process name (Task Manager → Details), with or without `.exe`. |
| `-BnetExe` | auto-detect | Path to `Battle.net.exe`, if auto-detection fails. |
| `-BnetMaxWait` / `-SettleDelay` | `20` / `3` | Seconds waiting for Battle.net's window, then settling, before sending the launch code. |
| `-RetryAfter` / `-StartupWait` | `30` / `120` | When to resend the launch code, and when to give up. |

Log/lock: `%TEMP%\start-bnet-game.log` / `.lock`, cleaned up automatically.

---

## Setup — SteamOS / Steam Deck / Steam Machine

1. **Install Battle.net in Desktop Mode first:** add the installer as a non-Steam game, force a Proton version (GE-Proton) under Properties → Compatibility, run it once to install Battle.net and your game.
2. **Don't remove that shortcut** — doing so can delete its whole Proton prefix (and your install with it). Instead, **retarget it**: right-click → Properties, set Target and Start In to the installed exe, found under:
   ```
   ~/.local/share/Steam/steamapps/compatdata/<AppID>/pfx/drive_c/Program Files (x86)/Battle.net/
   ```
   Both fields need quotes (the path has spaces). If you've lost the folder: `sudo find / -iname "battle.net.exe" -print 2>/dev/null`.
3. Save the script, e.g. `~/scripts/start-bnet-game.sh`, and `chmod +x` it.
4. Set **Launch Options**:
   ```
   BNET_LAUNCH_CODE=WoWF BNET_GAME_PROCESS=WowB.exe ~/scripts/start-bnet-game.sh %command%
   ```
   Order matters: env vars, then script path, then `%command%` last.

   **If the script's path contains a space, quote it** — Steam passes non-Steam Linux launch options through a real shell, so it splits on spaces exactly like any shell command would:
   ```
   BNET_LAUNCH_CODE=WoWF BNET_GAME_PROCESS=WowB.exe "/home/deck/Steam Tools/start-bnet-game.sh" %command%
   ```
   **Don't combine `~` with quotes** — `"~/Steam Tools/start-bnet-game.sh"` won't expand `~`, since quoting disables that expansion in the shell. Use a full path (as above) or `$HOME`, which does expand inside quotes: `"$HOME/Steam Tools/start-bnet-game.sh"`.
5. Set a real gamepad layout under Properties → Controller — non-Steam shortcuts don't get one by default, and Steam Input won't work without it.

| Variable | Default | Description |
|---|---|---|
| `BNET_LAUNCH_CODE` | required | Same as `-LaunchCode` above. |
| `BNET_GAME_PROCESS` | required | Same as `-GameProcess` above — find via `pgrep -fa <name>` while the game runs; look for the `C:\...` path. |
| `BNET_MAX_WAIT` / `BNET_SETTLE_DELAY` | `30` / `8` | Same idea as the Windows waits. |
| `BNET_RETRY_AFTER` / `BNET_STARTUP_WAIT` | `30` / `120` | Same idea as the Windows waits. |

Log/lock: `$XDG_RUNTIME_DIR/start-bnet-game.log` / `.lock` (usually `/run/user/1000/`), cleaned up automatically — including if Steam's Stop button kills the script mid-run.

---

## Behavior notes

- Battle.net is force-closed on start and on exit, on both platforms — interrupts in-progress downloads, and means Battle.net isn't open for other games/chat unless you open it yourself.
- On SteamOS, the launch code is sent via a direct `proton run` call rather than re-running Steam's whole container chain — doing the latter can spin up a separate sandboxed session that never reaches the already-running Battle.net. Cleanup there is correspondingly aggressive (kill, force-kill, then tear down the Wine session as a last resort).
- Cleanup runs on virtually any exit (normal, error, or signal) on SteamOS via a trap; on Windows it's a `finally` block, which can't catch a hard kill (e.g. Task Manager "End task") — a known, unfixable gap.
- A lock file stops two overlapping launches (e.g. double-pressing Play) from racing each other.
- If the game process is already running when the script starts (launched manually, or Play pressed while already in-game), Battle.net is left alone entirely — the script just waits on the existing process instead of restarting anything.
- Log lines are timestamped, so a slow step and a stuck one look different even without watching it live.

## Launch codes

Pass the code to `-LaunchCode` / `BNET_LAUNCH_CODE` (just the code, e.g. `Fen`, not the whole `--exec` string). Community-collected, not published by Blizzard — treat as a starting point and check the log if one doesn't work. Only **WoW Forever** (`WoWF`) has actually been tested with this script; everything else comes from the community sources below.

Last checked: 2026-09-21.

### Blizzard

| Game | Code | Notes |
|---|---|---|
| WoW Forever | `WoWF` | Tested with this script. |
| World of Warcraft | `WoW` | |
| WoW Classic (all versions) | `WoWC` | Version is whatever's selected in Battle.net's dropdown. |
| Diablo | `D1` | Includes Hellfire. |
| Diablo II: Resurrected | `OSI` | |
| Diablo III | `D3` | |
| Diablo IV | `Fen` | |
| Diablo Immortal (PC) | `ANBS` | |
| Hearthstone | `WTCG` | |
| Heroes of the Storm | `Hero` | |
| Overwatch | `Pro` | Formerly listed as Overwatch 2. |
| StarCraft | `S1` | Legacy/Remastered toggled in-game. |
| StarCraft II | `S2` | |
| Warcraft: Orcs & Humans | `W1` | |
| Warcraft II: Battle.net Edition | `W2` | |
| Warcraft: Remastered | `W1R` | |
| Warcraft II: Remastered | `W2R` | |
| Warcraft III: Reforged | `W3` | |
| Warcraft Rumble | `GRY` | |
| Blizzard Arcade Collection | `RTRO` | |

### Activision

| Game | Code | Notes |
|---|---|---|
| Call of Duty / Warzone / Black Ops 7 | `AUKS` | Titles switched inside the launcher. |
| Call of Duty: Black Ops 4 | `VIPR` | |
| Call of Duty: Black Ops 6 | `BTLR` | Standalone entry since a July 2026 update. |
| Call of Duty: Black Ops Cold War | `ZEUS` | |
| Call of Duty: Modern Warfare (2019) | `ODIN` | |
| Call of Duty: Modern Warfare II | `NINA` | |
| Call of Duty: Modern Warfare III | `PNTA` | |
| Call of Duty: MW2 Campaign Remastered | `LAZR` | |
| Call of Duty: Vanguard | `FORE` | |
| Crash Bandicoot 4: It's About Time | `WLBY` | |

### Other publishers on Battle.net

| Game | Code | Notes |
|---|---|---|
| Avowed | `AQUA` | |
| DOOM: The Dark Ages | `ARIS` | |
| The Outer Worlds 2 | `ARK` | |
| Sea of Thieves | `SCOR` | |
| Tony Hawk's Pro Skater 3 + 4 | `LBRA` | |
| The Witcher 3: Wild Hunt Remastered | `LYRA` | Listed before release; may not work yet. |

### Caveats

- A code may open the game's page instead of starting it — the script will time out and exit with code `2`.
- Not every game supports the Steam overlay (some Call of Duty titles have reported issues) or is confirmed to run under Proton (check ProtonDB).

### Sources

- Steam Community guide "Run Games from Battlenet Launcher with Steam Overlay" (<https://steamcommunity.com/sharedfiles/filedetails/?id=1113049716>)
- OriginSteamOverlayLauncher wiki, "Battle.net Launcher" (<https://github.com/WombatFromHell/OriginSteamOverlayLauncher/wiki/Battle.net-Launcher>)
- Community gist of Battle.net Steam launch options for Linux/Proton (<https://gist.github.com/kriegalex/4a74c19f8aefd7487ef87854306be93e>)

Found a new or changed code? Pull requests are welcome.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Blank Steam entry, nothing launches | You added the script directly as Target — add `powershell.exe` (Win) or the real `Battle.net.exe` (SteamOS), put the script in Launch Options. |
| "Both parameters are required" in the log | Add the launch code / process name to Launch Options. |
| Battle.net opens but game doesn't start | Raise `SettleDelay` (try 10-20 on SteamOS). |
| Status drops right after the game starts | `GameProcess` doesn't match the real process name — recheck it while running. |
| (Windows) "Could not find Battle.net.exe" | Pass `-BnetExe` explicitly. |
| (Windows) Nothing launches, script path has a space in it | Quote the `-File` path (e.g. anything under `Documents` or `Program Files`). |
| (SteamOS) Nothing launches, script path has a space in it | Quote the script path in Launch Options — and don't combine `~` with quotes, use `$HOME` or a full path instead. |
| (SteamOS) Removing the shortcut wiped the install | Retarget existing shortcuts, never remove once installed. |
| (SteamOS) Nothing launches despite a correct-looking Target | Target and Start In both need quotes. |
| (SteamOS) Steam Input doesn't work | Set a gamepad layout under Properties → Controller. |
| (SteamOS) Battle.net/status doesn't clear on exit | Confirm `wineserver` is on `PATH` — the last-resort cleanup step needs it. |
| (SteamOS) "Could not identify the Proton binary" | Confirm Launch Options still end in `%command%` and weren't edited. |

Exit codes (both): `0` normal, `1` error, `2` game never appeared.

## Disclaimer

Not affiliated with Blizzard, Battle.net, or Valve. Force-closes processes and, on SteamOS, can tear down a Wine session — use at your own risk.

## License

MIT, see [LICENSE](LICENSE).
