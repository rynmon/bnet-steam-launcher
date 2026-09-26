#!/usr/bin/env bash
# start-bnet-game.sh - Steam Deck / SteamOS / Linux + Proton equivalent of Start-BnetGame.ps1
#
# Add Battle.net.exe as a non-Steam game, force a Proton compatibility tool, then set
# Launch Options to:
#   BNET_LAUNCH_CODE=WoWF BNET_GAME_PROCESS=WowB.exe /path/to/start-bnet-game.sh %command%
#
# %command% expands to the full reaper/Proton chain Steam builds for this entry. This
# script runs that chain itself, waits for the real (Windows-side) Battle.net process,
# sends the launch code, waits for the game process, and stays alive until the game exits
# so Steam keeps the in-game status.
#
# Log: $XDG_RUNTIME_DIR/start-bnet-game.log (usually /run/user/1000/...), or /tmp if unset.
#
# IMPORTANT: every process in Steam's own launch chain (reaper, steam-launch-wrapper,
# proton, this script itself) has "Battle.net.exe" somewhere in ITS OWN command line too,
# since that path is just passed along as an argument. A plain `pgrep -f Battle.net.exe`
# matches all of those, including this script. Only the real Windows-side process (running
# under Wine) reports a "C:\..." path, so that's what every check here filters for.

set -u

LAUNCH_CODE="${BNET_LAUNCH_CODE:-}"
GAME_PROCESS="${BNET_GAME_PROCESS:-}"
BNET_WAIT="${BNET_MAX_WAIT:-30}"         # seconds to wait for the real Battle.net process before sending the launch code anyway
SETTLE_DELAY="${BNET_SETTLE_DELAY:-8}"   # extra seconds after Battle.net appears (raise if the launch code gets ignored)
RETRY_AFTER="${BNET_RETRY_AFTER:-30}"    # resend the launch code once if the game hasn't appeared after this many seconds
STARTUP_WAIT="${BNET_STARTUP_WAIT:-120}" # give up if the game process hasn't appeared after this many seconds

LOG="${XDG_RUNTIME_DIR:-/tmp}/start-bnet-game.log"
exec > "$LOG" 2>&1
echo "=== $(date) ==="

if [[ -z "$LAUNCH_CODE" || -z "$GAME_PROCESS" ]]; then
    echo "ERROR: set BNET_LAUNCH_CODE and BNET_GAME_PROCESS before %command% in Launch Options"
    exit 1
fi
if [[ $# -eq 0 ]]; then
    echo "ERROR: no command received - Launch Options must end with %command%"
    exit 1
fi
echo "Launch code: $LAUNCH_CODE | Game process: $GAME_PROCESS"
echo "Base command: $*"

# Finds the PID of the real Windows-side process (cmdline starts with "C:\..."), ignoring
# every Linux-side wrapper process that merely mentions the same name as an argument.
find_wine_pid() {
    pgrep -fa "$1" 2>/dev/null | awk '$2 ~ /^[Cc]:/ { print $1; exit }'
}

stop_bnet() {
    local pid
    pid="$(find_wine_pid 'Battle\.net\.exe')"
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null
}

# 1. Clean up any leftover (real) Battle.net from a previous/crashed run.
stop_bnet
sleep 2

# 2. Start Battle.net via the exact command Steam/Proton built for this entry.
"$@" &
bnet_job=$!

# 3. Wait for the real Battle.net process (not the wrapper chain) to appear, then settle.
deadline=$(( $(date +%s) + BNET_WAIT ))
while [[ $(date +%s) -lt $deadline ]]; do
    [[ -n "$(find_wine_pid 'Battle\.net\.exe')" ]] && break
    sleep 0.5
done
if [[ -n "$(find_wine_pid 'Battle\.net\.exe')" ]]; then
    echo "Battle.net process seen"
else
    echo "No Battle.net process seen after ${BNET_WAIT}s, continuing anyway"
fi
sleep "$SETTLE_DELAY"

# 4. Send the launch code by invoking the same command again with --exec.
#    Battle.net is single-instance, so this should hand off to the running copy and exit.
echo "Sending launch command"
"$@" --exec="launch $LAUNCH_CODE"

# 5. Wait for the game process; resend once if it hasn't appeared after $RETRY_AFTER seconds.
deadline=$(( $(date +%s) + STARTUP_WAIT ))
retry_at=$(( $(date +%s) + RETRY_AFTER ))
retried=0
game_pid=""
while [[ $(date +%s) -lt $deadline ]]; do
    game_pid="$(find_wine_pid "$GAME_PROCESS")"
    [[ -n "$game_pid" ]] && break
    if [[ $retried -eq 0 && $(date +%s) -gt $retry_at ]]; then
        echo "Game not seen yet, resending launch command"
        "$@" --exec="launch $LAUNCH_CODE"
        retried=1
    fi
    sleep 1
done

# 6. Stay alive until the game exits, so Steam keeps the in-game status.
if [[ -n "$game_pid" ]]; then
    echo "Game running (PID $game_pid), waiting for exit"
    while kill -0 "$game_pid" 2>/dev/null; do sleep 2; done
    echo "Game exited"
    exit_code=0
else
    echo "Game process '$GAME_PROCESS' never appeared within ${STARTUP_WAIT}s"
    exit_code=2
fi

# 7. Close Battle.net so Steam clears the status.
stop_bnet
wait "$bnet_job" 2>/dev/null

exit "$exit_code"