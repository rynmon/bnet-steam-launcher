# bnet-steam-launcher

Launch a **Battle.net game** (built and tested with **WoW Forever**) from Steam and keep Steam's **in-game status**, **overlay** and **Steam Input** working. When you exit the game, the Steam status clears.

## The problem

Adding `Battle.net.exe --exec="launch WoWF"` as a non-Steam shortcut doesn't work well:

- Steam shows "In-game" only while the process it launched is alive. The `--exec` command hands off to Battle.net and returns, so Steam sees the "game" exit within seconds and drops the status.
- The Steam overlay and Steam Input attach to processes that descend from the Steam launch. If Battle.net was already running (for example from Windows startup), the game is started by that older instance, so it never gets them.

## What this script does

1. Closes any running Battle.net and starts a fresh one **from the Steam launch**, so the game becomes a child of it.
2. Waits for the Battle.net window to appear, then sends `--exec="launch <code>"`.
3. Waits for the game process, resending the launch command once if it doesn't show up.
4. Stays alive until the game exits, so Steam keeps the in-game status.
5. Closes Battle.net so Steam clears the status.

## Requirements

- Windows with PowerShell 5.1 or later (built in)
- Steam and the Battle.net desktop app
- **Tested with WoW Forever only.** Other Battle.net games use the same `--exec="launch <code>"` mechanism and should work, but haven't been tested.

## Setup

1. Save `Start-BnetGame.ps1` somewhere permanent, e.g. `C:\Scripts\Start-BnetGame.ps1`.
2. In Steam: **Games → Add a Non-Steam Game to My Library → Browse**, set the file filter to **All files**, and add PowerShell:

   ```
   C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
   ```

   Don't add the `.ps1` directly. Steam can't run it and creates a blank entry.
3. In your library, right-click the new entry → **Properties**:
   - Rename it (e.g. "WoW Forever").
   - Set **Launch Options** to the following. For WoW Forever:

     ```
     -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "C:\Scripts\Start-BnetGame.ps1" -LaunchCode WoWF -GameProcess WowB
     ```
4. Optional: set your controller layout in the same Properties window.
5. Optional: remove Battle.net from your Windows startup items. It isn't required, since the script restarts Battle.net anyway, but it's redundant.

Each game gets its own Steam entry with its own `-LaunchCode` and `-GameProcess`.

## Parameters

| Parameter | Default | Description |
|---|---|---|
| `-LaunchCode` | **required** | Code used in Battle.net's `--exec="launch <code>"`, e.g. `WoWF`. |
| `-GameProcess` | **required** | Game process name, with or without `.exe`, e.g. `WowB`. |
| `-BnetExe` | auto-detect | Full path to `Battle.net.exe`. Detected from the registry and default install paths when omitted. |
| `-BnetMaxWait` | `20` | Max seconds to wait for the Battle.net window before sending the launch command anyway. |
| `-SettleDelay` | `3` | Extra seconds to wait after the window appears. Raise it if the first launch command is ignored. |
| `-RetryAfter` | `30` | Resend the launch command once if the game hasn't appeared after this many seconds. |
| `-StartupWait` | `120` | Give up if the game process hasn't appeared after this many seconds. |

### Finding the values for a game

- **Launch code:** look the game up in [Launch codes](#launch-codes) below, or reuse the value from an existing `--exec="launch <code>"` shortcut.
- **Game process:** start the game normally, open Task Manager → **Details**, and find its `.exe` name.

## Launch codes

Pass the code to `-LaunchCode` (just the code, e.g. `Fen`, not the whole `--exec` string). These are community-collected and Blizzard doesn't publish them, so treat the list as a starting point. Codes change when Battle.net adds or reworks games.

**Status column:**
- **Verified**: tested with this script.
- **Multiple sources**: listed independently by two or more community sources.
- **Single source**: listed by one source only, so try it and check the log.

Last checked: 2026-09-21.

### Blizzard

| Game | Code | Status | Notes |
|---|---|---|---|
| WoW Forever | `WoWF` | Verified | |
| World of Warcraft | `WoW` | Multiple sources | |
| WoW Classic (all Classic versions) | `WoWC` | Multiple sources | Which Classic version starts is whatever is selected in Battle.net's game dropdown. |
| Diablo | `D1` | Single source | Includes the Hellfire expansion. |
| Diablo II: Resurrected | `OSI` | Single source | |
| Diablo III | `D3` | Multiple sources | |
| Diablo IV | `Fen` | Multiple sources | |
| Diablo Immortal (PC) | `ANBS` | Single source | |
| Hearthstone | `WTCG` | Multiple sources | |
| Heroes of the Storm | `Hero` | Multiple sources | |
| Overwatch | `Pro` | Multiple sources | Formerly listed as Overwatch 2. |
| StarCraft | `S1` | Multiple sources | Legacy and Remastered are toggled in-game. |
| StarCraft II | `S2` | Multiple sources | |
| Warcraft: Orcs & Humans | `W1` | Single source | |
| Warcraft II: Battle.net Edition | `W2` | Single source | |
| Warcraft: Remastered | `W1R` | Single source | |
| Warcraft II: Remastered | `W2R` | Single source | |
| Warcraft III: Reforged | `W3` | Multiple sources | |
| Warcraft Rumble | `GRY` | Single source | |
| Blizzard Arcade Collection | `RTRO` | Single source | |

### Activision

| Game | Code | Status | Notes |
|---|---|---|---|
| Call of Duty / Warzone / Black Ops 7 | `AUKS` | Single source | These titles are switched inside the launcher. |
| Call of Duty: Black Ops 4 | `VIPR` | Multiple sources | |
| Call of Duty: Black Ops 6 | `BTLR` | Single source | Reported as a standalone entry since a July 2026 update. |
| Call of Duty: Black Ops Cold War | `ZEUS` | Multiple sources | |
| Call of Duty: Modern Warfare (2019) | `ODIN` | Multiple sources | |
| Call of Duty: Modern Warfare II | `NINA` | Single source | |
| Call of Duty: Modern Warfare III | `PNTA` | Single source | |
| Call of Duty: MW2 Campaign Remastered | `LAZR` | Multiple sources | |
| Call of Duty: Vanguard | `FORE` | Single source | |
| Crash Bandicoot 4: It's About Time | `WLBY` | Single source | |

### Other publishers on Battle.net

| Game | Code | Status | Notes |
|---|---|---|---|
| Avowed | `AQUA` | Single source | |
| DOOM: The Dark Ages | `ARIS` | Single source | |
| The Outer Worlds 2 | `ARK` | Single source | |
| Sea of Thieves | `SCOR` | Single source | |
| Tony Hawk's Pro Skater 3 + 4 | `LBRA` | Single source | |
| The Witcher 3: Wild Hunt Remastered | `LYRA` | Single source | Listed before release; may not work yet. |

### Caveats

- **Only WoW Forever has been tested with this script.** Every other code comes from the sources below.
- **A code may open the game's page instead of starting the game**, depending on the game and launcher version. If so, this script will wait, log that the game process never appeared, and exit with code `2`.
- **Not every game supports the Steam overlay.** Some Call of Duty titles have community-reported overlay problems.
- **Process names aren't listed here.** Find each game's `-GameProcess` in Task Manager, as described above.

### Sources

- Steam Community guide "Run Games from Battlenet Launcher with Steam Overlay" (<https://steamcommunity.com/sharedfiles/filedetails/?id=1113049716>)
- OriginSteamOverlayLauncher wiki, "Battle.net Launcher" (<https://github.com/WombatFromHell/OriginSteamOverlayLauncher/wiki/Battle.net-Launcher>)
- A community gist of Battle.net Steam launch options for Linux/Proton (<https://gist.github.com/kriegalex/4a74c19f8aefd7487ef87854306be93e>)

Found a new or changed code? Pull requests are welcome.

## Behavior and limitations

- **Battle.net is force-closed** when the script starts and again when the game exits. If you launch during a download or patch, it will be interrupted. Battle.net won't be open after you quit the game, so open it yourself for shop, chat or other games.
- **The overlay depends on the restart.** In testing, reusing an already-running Battle.net kept the status but the overlay didn't work, which is why the script always restarts it.
- **Game updates:** if the game needs patching, the launch command may not start it. Launch it normally from Battle.net once to update, then use Steam again.
- **Windows only.**

## Troubleshooting

The script logs each step to `%TEMP%\Start-BnetGame.log`. The file is overwritten on each run.

| Symptom | Likely fix |
|---|---|
| Battle.net opens but the game doesn't start | Raise `-SettleDelay` (try 6-10). The log shows whether the retry was needed. |
| Game starts but the Steam status drops shortly after | `-GameProcess` doesn't match the real process name. Check Task Manager → Details. |
| Log says both parameters are required | Add `-LaunchCode` and `-GameProcess` to the Steam launch options. |
| Log says `Could not find Battle.net.exe` | Pass the full path with `-BnetExe`. |
| Steam shows a blank entry and nothing launches | You added the `.ps1` directly. Add `powershell.exe` and use Launch Options instead. |

Exit codes: `0` normal, `1` error (e.g. missing parameters or Battle.net not found), `2` the game never appeared.

## Disclaimer

This isn't affiliated with Blizzard, Battle.net or Valve. It force-closes a process and is provided as-is, so use it at your own risk.

## License

MIT, see [LICENSE](LICENSE).
