# bnet-steam-launcher

Launch a **Battle.net game** (built and tested with **WoW Forever**) from Steam and keep Steam's **in-game status**, **overlay** and **Steam Input** working. When you exit the game, the Steam status clears.

Works on **Windows** (PowerShell) and on **SteamOS** — Steam Deck and Steam Machine (Bash + Proton). Tested end-to-end on all three.

## The problem

Adding `Battle.net.exe --exec="launch WoWF"` as a non-Steam shortcut doesn't work well:

- Steam shows "In-game" only while the process it launched is alive. The `--exec` command hands off to Battle.net and returns, so Steam sees the "game" exit within seconds and drops the status.
- The Steam overlay and Steam Input attach to processes that descend from the Steam launch. If Battle.net was already running (for example from Windows startup), the game is started by that older instance, so it never gets them.
- On SteamOS, Battle.net has no native client and has to run under Proton, and re-sending a launch command by simply re-running the whole Steam/Proton launch chain a second time can hang or silently fail (see [Behavior and limitations](#behavior-and-limitations)).

## What this does

1. Closes any running Battle.net and starts a fresh one **from the Steam launch**, so the game becomes a child of it.
2. Waits for Battle.net to actually be up, then sends `--exec="launch <code>"`.
3. Waits for the game process, resending the launch command once if it doesn't show up.
4. Stays alive until the game exits, so Steam keeps the in-game status.
5. Closes Battle.net so Steam clears the status.

Two implementations do this: `Start-BnetGame.ps1` (Windows, PowerShell) and `start-bnet-game.sh` (SteamOS/Linux, Bash + Proton). Pick the one for your platform below.

## Requirements

- **Windows:** PowerShell 5.1 or later (built in), Steam, the Battle.net desktop app.
- **SteamOS / Steam Deck / Steam Machine:** Steam, the Windows Battle.net installer (run once under Proton — there's no native Linux client), a Proton compatibility tool (GE-Proton is commonly recommended for Battle.net).
- **Tested with WoW Forever only.** Other Battle.net games use the same `--exec="launch <code>"` mechanism and should work, but haven't been tested. See [Launch codes](#launch-codes).

---

## Setup on Windows

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
4. Set your controller layout in the same Properties window if you use one.
5. Optional: remove Battle.net from your Windows startup items. It isn't required, since the script restarts Battle.net anyway, but it's redundant.

Each game gets its own Steam entry with its own `-LaunchCode` and `-GameProcess`.

### Windows parameters

| Parameter | Default | Description |
|---|---|---|
| `-LaunchCode` | **required** | Code used in Battle.net's `--exec="launch <code>"`, e.g. `WoWF`. |
| `-GameProcess` | **required** | Game process name, with or without `.exe`, e.g. `WowB`. |
| `-BnetExe` | auto-detect | Full path to `Battle.net.exe`. Detected from the registry and default install paths when omitted. |
| `-BnetMaxWait` | `20` | Max seconds to wait for the Battle.net window before sending the launch command anyway. |
| `-SettleDelay` | `3` | Extra seconds to wait after the window appears. Raise it if the first launch command is ignored. |
| `-RetryAfter` | `30` | Resend the launch command once if the game hasn't appeared after this many seconds. |
| `-StartupWait` | `120` | Give up if the game process hasn't appeared after this many seconds. |

Log: `%TEMP%\Start-BnetGame.log` (overwritten each run).

---

## Setup on SteamOS / Steam Deck / Steam Machine

1. **Install Battle.net first, in Desktop Mode.** Add the Battle.net Windows installer as a non-Steam game (Browse → All Files), force a Proton version (GE-Proton recommended) in **Properties → Compatibility**, and run it once to install Battle.net and your game.
2. **Don't remove that shortcut when you're done.** Removing a non-Steam entry can delete its entire Proton prefix — Battle.net and the game install *inside* it, so removing the shortcut wipes them out too. Instead, **retarget the same entry**:
   - Right-click it → **Properties**.
   - Set **Target** and **Start In** to the installed `Battle.net.exe`, found under:
     ```
     ~/.local/share/Steam/steamapps/compatdata/<AppID>/pfx/drive_c/Program Files (x86)/Battle.net/
     ```
     If you've lost track of the folder, search for it:
     ```
     sudo find / -iname "battle.net.exe" -print 2>/dev/null
     ```
   - **Both Target and Start In need quotes** around the path — it contains spaces (`Program Files (x86)`), and Steam won't run it without them. Start In should be the *folder*, Target the *exe*.
3. **Save the script** somewhere permanent, e.g. `~/scripts/start-bnet-game.sh`, and make it executable:
   ```
   chmod +x ~/scripts/start-bnet-game.sh
   ```
4. **Set Launch Options** on the same entry. For WoW Forever:
   ```
   BNET_LAUNCH_CODE=WoWF BNET_GAME_PROCESS=WowB.exe ~/scripts/start-bnet-game.sh %command%
   ```
   Order matters: environment variables first, then the script path, then `%command%` last. `%command%` carries Steam's Proton invocation into the script, so the script can run it itself.
5. **Set the controller layout** in **Properties → Controller**. Non-Steam shortcuts don't get a gamepad profile by default — pick a real layout (e.g. "Gamepad with Joystick Trackpad") or Steam Input won't work even though the script is otherwise fine.
6. Test it. See [Troubleshooting](#troubleshooting) if anything doesn't come up.

### Finding the game process name (both platforms)

Start the game normally (manually, from inside Battle.net), then:
- **Windows:** Task Manager → **Details** tab.
- **SteamOS:** `pgrep -fa <part of the name>` in a terminal, e.g. `pgrep -fa wow`. Look for the real process, reported with a Windows-style path (`C:\Program Files (x86)\...\WowB.exe`).

### SteamOS environment variables

| Variable | Default | Description |
|---|---|---|
| `BNET_LAUNCH_CODE` | **required** | Code used in Battle.net's `--exec="launch <code>"`, e.g. `WoWF`. |
| `BNET_GAME_PROCESS` | **required** | Game process name, with or without `.exe`, e.g. `WowB.exe`. |
| `BNET_MAX_WAIT` | `30` | Max seconds to wait for the real Battle.net process before sending the launch code anyway. |
| `BNET_SETTLE_DELAY` | `8` | Extra seconds after Battle.net appears. Raise it if the first launch code is ignored. |
| `BNET_RETRY_AFTER` | `30` | Resend the launch code once if the game hasn't appeared after this many seconds. |
| `BNET_STARTUP_WAIT` | `120` | Give up if the game process hasn't appeared after this many seconds. |

Log: `$XDG_RUNTIME_DIR/start-bnet-game.log`, usually `/run/user/1000/start-bnet-game.log`.

---

## Launch codes

Pass the code to `-LaunchCode` / `BNET_LAUNCH_CODE` (just the code, e.g. `Fen`, not the whole `--exec` string). These are community-collected and Blizzard doesn't publish them, so treat the list as a starting point. Codes change when Battle.net adds or reworks games.

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
- **Not every game is confirmed to run under Proton on SteamOS**, particularly anti-cheat-protected titles. Check ProtonDB before assuming it'll work there.
- **Process names aren't listed here.** Find each game's process name as described above.

### Sources

- Steam Community guide "Run Games from Battlenet Launcher with Steam Overlay" (<https://steamcommunity.com/sharedfiles/filedetails/?id=1113049716>)
- OriginSteamOverlayLauncher wiki, "Battle.net Launcher" (<https://github.com/WombatFromHell/OriginSteamOverlayLauncher/wiki/Battle.net-Launcher>)
- A community gist of Battle.net Steam launch options for Linux/Proton (<https://gist.github.com/kriegalex/4a74c19f8aefd7487ef87854306be93e>)

Found a new or changed code? Pull requests are welcome.

## Behavior and limitations

- **Battle.net is force-closed** when the script starts and again when the game exits, on both platforms. If you launch during a download or patch, it will be interrupted. Battle.net won't be open after you quit the game, so open it yourself for shop, chat or other games.
- **The overlay depends on the restart.** Reusing an already-running Battle.net kept the status but not the overlay/Steam Input, which is why the script always restarts it.
- **On SteamOS specifically, sending the launch code avoids re-running Steam's whole container/Proton chain.** Doing that a second time can spin up a separately-sandboxed session that doesn't share the first one's Wine server, so Battle.net's single-instance handoff silently fails or hangs. Instead, the script extracts the actual Proton binary from the base command and calls `proton run <exe> --exec=...` directly, which stays in the same session and doesn't wait for exit.
- **Cleanup on SteamOS is deliberately aggressive:** every matching Battle.net process is killed, then force-killed if still alive after a couple seconds, then as a last resort the entire Wine prefix's session is torn down (`wineserver -k`) so Steam is guaranteed to see the launch end.
- **Game updates:** if the game needs patching, the launch command may not start it. Launch it normally from Battle.net once to update, then use Steam again.
- **Windows and SteamOS only.**

## Troubleshooting

| Symptom | Platform | Likely fix |
|---|---|---|
| Steam shows a blank entry and nothing launches | Both | You added the script/`.ps1` directly as the Target. Add `powershell.exe` (Windows) or the real `Battle.net.exe` (SteamOS) as Target, and put the script in Launch Options instead. |
| Log says both parameters are required | Both | Add `-LaunchCode`/`-GameProcess` (Windows) or `BNET_LAUNCH_CODE`/`BNET_GAME_PROCESS` (SteamOS) to the Launch Options. |
| Battle.net opens but the game doesn't start | Both | Raise `-SettleDelay` / `BNET_SETTLE_DELAY` (try 10-20 on SteamOS). The log shows whether the retry was needed. |
| Game starts but the Steam status drops shortly after | Both | `-GameProcess` / `BNET_GAME_PROCESS` doesn't match the real process name. Re-check it while the game is running. |
| Log says `Could not find Battle.net.exe` | Windows | Pass the full path with `-BnetExe`. |
| Removing/renaming the shortcut wiped the install | SteamOS | Don't remove non-Steam entries once Battle.net is installed — retarget the existing one instead (see Setup). |
| Target field looks right but nothing launches | SteamOS | Target and Start In both need quotes around the path (it contains spaces). |
| Script doesn't run at all from Steam, no log activity | SteamOS | Check `chmod +x` on the script, and that the script path in Launch Options is correct and comes *before* `%command%`, with env vars before the script path. |
| Steam Input doesn't work even though the game runs fine | SteamOS | Set a real gamepad layout in **Properties → Controller** — non-Steam shortcuts don't get one by default. |
| Battle.net stays open / Steam status doesn't clear after exiting the game | SteamOS | Should be fixed by the current script's aggressive cleanup. If it still happens, confirm `wineserver` is on `PATH` (`which wineserver`) — if not, the last-resort cleanup step silently does nothing. |
| Log says `Could not identify the Proton binary` | SteamOS | The script falls back to resending the full command, which is slower and may hang. Check that your Launch Options end in `%command%` and haven't been edited. |

Exit codes (both platforms): `0` normal, `1` error (e.g. missing parameters or Battle.net not found), `2` the game never appeared.

## Disclaimer

This isn't affiliated with Blizzard, Battle.net or Valve. It force-closes processes and, on SteamOS, can tear down a Wine session — provided as-is, use at your own risk.

## License

MIT, see [LICENSE](LICENSE).
