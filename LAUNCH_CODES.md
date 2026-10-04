# Launch codes

Pass the code to `-LaunchCode` / `BNET_LAUNCH_CODE` (just the code, e.g. `Fen`, not the whole `--exec` string). Community-collected, not published by Blizzard — treat as a starting point and check the log if one doesn't work.

**Status:** Verified (tested with this script) · Multiple sources · Single source (try it and check the log).

Last checked: 2026-09-21.

### Blizzard

| Game | Code | Status | Notes |
|---|---|---|---|
| WoW Forever | `WoWF` | Verified | |
| World of Warcraft | `WoW` | Multiple sources | |
| WoW Classic (all versions) | `WoWC` | Multiple sources | Version is whatever's selected in Battle.net's dropdown. |
| Diablo | `D1` | Single source | Includes Hellfire. |
| Diablo II: Resurrected | `OSI` | Single source | |
| Diablo III | `D3` | Multiple sources | |
| Diablo IV | `Fen` | Multiple sources | |
| Diablo Immortal (PC) | `ANBS` | Single source | |
| Hearthstone | `WTCG` | Multiple sources | |
| Heroes of the Storm | `Hero` | Multiple sources | |
| Overwatch | `Pro` | Multiple sources | Formerly listed as Overwatch 2. |
| StarCraft | `S1` | Multiple sources | Legacy/Remastered toggled in-game. |
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
| Call of Duty / Warzone / Black Ops 7 | `AUKS` | Single source | Titles switched inside the launcher. |
| Call of Duty: Black Ops 4 | `VIPR` | Multiple sources | |
| Call of Duty: Black Ops 6 | `BTLR` | Single source | Standalone entry since a July 2026 update. |
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

- Only WoW Forever has been tested with this script; every other code is from the sources below.
- A code may open the game's page instead of starting it — the script will time out and exit with code `2`.
- Not every game supports the Steam overlay (some Call of Duty titles have reported issues) or is confirmed to run under Proton (check ProtonDB).

### Sources

- Steam Community guide "Run Games from Battlenet Launcher with Steam Overlay" (<https://steamcommunity.com/sharedfiles/filedetails/?id=1113049716>)
- OriginSteamOverlayLauncher wiki, "Battle.net Launcher" (<https://github.com/WombatFromHell/OriginSteamOverlayLauncher/wiki/Battle.net-Launcher>)
- Community gist of Battle.net Steam launch options for Linux/Proton (<https://gist.github.com/kriegalex/4a74c19f8aefd7487ef87854306be93e>)

Found a new or changed code? Pull requests are welcome.
