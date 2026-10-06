#!/usr/bin/env bash
# start-bnet-game.sh - Steam Deck / SteamOS / Linux + Proton equivalent of start-bnet-game.ps1
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
# Log:  $XDG_RUNTIME_DIR/start-bnet-game.log (usually /run/user/1000/...), or /tmp if unset.
# Lock: $XDG_RUNTIME_DIR/start-bnet-game.lock - prevents two overlapping runs (e.g. Play
#       pressed twice) from racing each other's cleanup.
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
RETRY_AFTER="${BNET_RETRY_AFTER:-10}"    # resend the launch code every N seconds until the game appears (not just once)
STARTUP_WAIT="${BNET_STARTUP_WAIT:-120}" # give up if the game process hasn't appeared after this many seconds

RUN_DIR="${XDG_RUNTIME_DIR:-/tmp}"
LOG="$RUN_DIR/start-bnet-game.log"
LOCK="$RUN_DIR/start-bnet-game.lock"
exec > "$LOG" 2>&1
log() { echo "[$(date +%T)] $*"; }
log "=== run started $(date) ==="

# --- Single-instance guard -------------------------------------------------
# If another copy of this script is already running (e.g. Play pressed twice),
# exit immediately rather than racing its launch/cleanup steps.
if [[ -f "$LOCK" ]]; then
    old_pid="$(cat "$LOCK" 2>/dev/null)"
    if [[ -n "$old_pid" ]] && kill -0 "$old_pid" 2>/dev/null; then
        log "Another instance (PID $old_pid) is already running - exiting"
        exit 1
    fi
fi
echo $$ > "$LOCK"

if [[ -z "$LAUNCH_CODE" || -z "$GAME_PROCESS" ]]; then
    log "ERROR: set BNET_LAUNCH_CODE and BNET_GAME_PROCESS before %command% in Launch Options"
    rm -f "$LOCK"
    exit 1
fi
if [[ $# -eq 0 ]]; then
    log "ERROR: no command received - Launch Options must end with %command%"
    rm -f "$LOCK"
    exit 1
fi
log "Launch code: $LAUNCH_CODE | Game process: $GAME_PROCESS"
log "Base command: $*"

# Pull the actual Proton binary and the target exe out of the base command, so the
# launch code can be sent via a direct, lightweight "proton run" call afterwards instead
# of re-running Steam's whole container/runtime chain a second time. Re-running the full
# chain spins up a second, separately-sandboxed session that may not share the first
# session's Wine server, so Battle.net's single-instance handoff never happens and the
# second invocation just hangs or silently does nothing.
proton_bin=""
bnet_exe="${!#}"   # last argument in the base command is the target exe path
for arg in "$@"; do
    case "$arg" in
        */proton) proton_bin="$arg" ;;
    esac
done
# The Wine prefix root is everything before "/drive_c/" in the target exe path.
# Used as a last-resort way to fully end the Wine session on cleanup.
wineprefix="${bnet_exe%%/drive_c/*}"
if [[ -n "$proton_bin" && -x "$proton_bin" ]]; then
    log "Proton binary: $proton_bin"
    log "Target exe: $bnet_exe"
else
    log "Could not identify the Proton binary from the base command - will fall back to resending the full command"
fi

# Args passed here are only used by the fallback path.
send_launch_code() {
    if [[ -n "$proton_bin" && -x "$proton_bin" ]]; then
        "$proton_bin" run "$bnet_exe" --exec="launch $LAUNCH_CODE" &
        disown
    else
        "$@" --exec="launch $LAUNCH_CODE" &
        disown
    fi
}

# Finds the PID of the real Windows-side process (cmdline starts with "C:\..."), ignoring
# every Linux-side wrapper process that merely mentions the same name as an argument.
find_wine_pid() {
    pgrep -fia "$1" 2>/dev/null | awk '$2 ~ /^[Cc]:/ { print $1; exit }'
}

# Kill every real Battle.net process (main + helpers), then force-kill anything left,
# then as a last resort tear down the whole Wine session for this prefix - guarantees
# Proton's wrapper (and therefore Steam's tracked launch) actually ends.
stop_bnet() {
    local pids
    pids="$(pgrep -fia 'Battle\.net\.exe' 2>/dev/null | awk '$2 ~ /^[Cc]:/ { print $1 }')"
    [[ -n "$pids" ]] && kill $pids 2>/dev/null
    sleep 2
    pids="$(pgrep -fia 'Battle\.net\.exe' 2>/dev/null | awk '$2 ~ /^[Cc]:/ { print $1 }')"
    [[ -n "$pids" ]] && kill -9 $pids 2>/dev/null
    if [[ -n "$wineprefix" && -d "$wineprefix" ]]; then
        WINEPREFIX="$wineprefix" wineserver -k 2>/dev/null
    fi
}

# Runs on any exit - normal completion, an error, or Steam/the user killing the script
# (e.g. the Stop button). Without this, a non-normal exit could leave Battle.net running
# and Steam's in-game status stuck on, the same symptom a slow/failed launch used to cause.
cleanup() {
    stop_bnet
    rm -f "$LOCK" 2>/dev/null
}
trap cleanup EXIT INT TERM HUP

# 0. If the game is already running (started manually, or Play pressed again by mistake),
#    don't touch Battle.net at all - just pick up tracking the existing session.
game_pid="$(find_wine_pid "$GAME_PROCESS")"
if [[ -n "$game_pid" ]]; then
    log "Game process '$GAME_PROCESS' already running (PID $game_pid) - skipping Battle.net restart"
else
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
        log "Battle.net process seen"
    else
        log "No Battle.net process seen after ${BNET_WAIT}s, continuing anyway"
    fi
    sleep "$SETTLE_DELAY"

    # 4. Send the launch code.
    log "Sending launch command"
    send_launch_code "$@"

    # 5. Wait for the game process; resend the launch code every $RETRY_AFTER seconds until
    #    it appears or $STARTUP_WAIT runs out, rather than giving up after a single resend.
    #    A silently-ignored first attempt now costs one retry interval, not the full timeout.
    deadline=$(( $(date +%s) + STARTUP_WAIT ))
    next_retry=$(( $(date +%s) + RETRY_AFTER ))
    while [[ $(date +%s) -lt $deadline ]]; do
        game_pid="$(find_wine_pid "$GAME_PROCESS")"
        [[ -n "$game_pid" ]] && break
        now=$(date +%s)
        if [[ $now -ge $next_retry ]]; then
            log "Game not seen yet, resending launch command"
            send_launch_code "$@"
            next_retry=$(( now + RETRY_AFTER ))
        fi
        sleep 1
    done
fi

# 6. Stay alive until the game exits, so Steam keeps the in-game status.
if [[ -n "$game_pid" ]]; then
    log "Game running (PID $game_pid), waiting for exit"
    while kill -0 "$game_pid" 2>/dev/null; do sleep 1; done
    log "Game exited"
    exit_code=0
else
    log "Game process '$GAME_PROCESS' never appeared within ${STARTUP_WAIT}s"
    exit_code=2
fi

# 7. Cleanup (closing Battle.net, removing the lock) happens automatically via the EXIT trap.
[[ -n "${bnet_job:-}" ]] && wait "$bnet_job" 2>/dev/null
exit "$exit_code"
