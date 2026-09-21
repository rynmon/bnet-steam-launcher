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

- **Launch code:** the value in your existing `--exec="launch <code>"` shortcut for that game.
- **Game process:** start the game normally, open Task Manager → **Details**, and find its `.exe` name.

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
