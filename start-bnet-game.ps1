<#
.SYNOPSIS
    Launch a Battle.net game from a Steam non-Steam shortcut, keeping Steam's in-game status,
    overlay and Steam Input working.

.DESCRIPTION
    Steam only shows "In-game" while the process it launched is alive, and the overlay/Steam Input
    only attach to processes that descend from that launch. Sending "--exec=launch <code>" to
    Battle.net directly returns immediately and the game is started by a separate Battle.net
    instance, so both break.

    This script (re)starts Battle.net itself (so the game becomes a child of the Steam launch),
    sends the launch command, waits for the game process, and stays alive until the game exits.
    Battle.net is closed when the game exits so Steam clears the in-game status.

    A lock file prevents two overlapping runs (e.g. Play pressed twice) from racing each other.

    Log: %TEMP%\<script name>.log

.PARAMETER LaunchCode
    REQUIRED. Game code used in Battle.net's --exec="launch <code>" (e.g. WoWF for WoW Forever).

.PARAMETER GameProcess
    REQUIRED. Game process name, with or without .exe (see Task Manager > Details).

.PARAMETER BnetExe
    Full path to Battle.net.exe. Auto-detected when omitted.

.PARAMETER BnetMaxWait
    Max seconds to wait for the Battle.net window before sending the launch command anyway.

.PARAMETER SettleDelay
    Extra seconds to wait after the Battle.net window appears. Raise if the first launch command is ignored.

.PARAMETER RetryAfter
    Resend the launch command every this-many seconds until the game appears (not just once).

.PARAMETER StartupWait
    Give up if the game process hasn't appeared after this many seconds.

.EXAMPLE
    # Steam launch options for WoW Forever:
    -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "C:\Scripts\start-bnet-game.ps1" -LaunchCode WoWF -GameProcess WowB
#>
[CmdletBinding()]
param(
    [string]$LaunchCode,
    [string]$GameProcess,
    [string]$BnetExe,
    [int]$BnetMaxWait = 20,
    [int]$SettleDelay = 3,
    [int]$RetryAfter  = 10,
    [int]$StartupWait = 120
)

$logName = if ($PSCommandPath) { [IO.Path]::GetFileNameWithoutExtension($PSCommandPath) } else { "start-bnet-game" }
Start-Transcript -Path (Join-Path $env:TEMP "$logName.log") -Force | Out-Null

$lockFile    = Join-Path $env:TEMP "$logName.lock"
$createdLock = $false

function Resolve-BnetExe {
    param([string]$Override)

    if ($Override) {
        if (Test-Path -LiteralPath $Override) { return $Override }
        throw "Battle.net not found at the path given in -BnetExe: $Override"
    }

    $candidates = @()

    # Installer registry entries (32-bit view first, then native)
    foreach ($key in @(
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Battle.net',
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Battle.net'
    )) {
        $loc = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).InstallLocation
        if ($loc) { $candidates += (Join-Path $loc 'Battle.net.exe') }
    }

    # Default install locations
    $candidates += (Join-Path ${env:ProgramFiles(x86)} 'Battle.net\Battle.net.exe')
    $candidates += (Join-Path $env:ProgramFiles        'Battle.net\Battle.net.exe')

    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { return $c }
    }
    throw "Could not find Battle.net.exe. Pass its full path with -BnetExe."
}

function Write-Log { param([string]$Message) Write-Output "[$(Get-Date -Format 'HH:mm:ss')] $Message" }
function Get-Game    { Get-Process -Name $GameProcess -ErrorAction SilentlyContinue | Select-Object -First 1 }
function Send-Launch { Start-Process -FilePath $BnetExe -ArgumentList "--exec=`"launch $LaunchCode`"" }
function Stop-Bnet {
    Get-Process -Name "Battle.net" -ErrorAction SilentlyContinue | Stop-Process -Force
    Wait-Process -Name "Battle.net" -Timeout 5 -ErrorAction SilentlyContinue
}
function Test-BnetWindow {
    [bool](Get-Process -Name "Battle.net" -ErrorAction SilentlyContinue |
           Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero })
}

$exitCode    = 0
$startedBnet = $false

try {
    # Single-instance guard: if another copy is already running (e.g. Play pressed twice),
    # exit immediately rather than racing its launch/cleanup steps.
    if (Test-Path -LiteralPath $lockFile) {
        $existingPid = Get-Content -LiteralPath $lockFile -ErrorAction SilentlyContinue
        if ($existingPid -and (Get-Process -Id $existingPid -ErrorAction SilentlyContinue)) {
            Write-Log "Another instance (PID $existingPid) is already running - exiting"
            exit 1
        }
    }
    $PID | Out-File -FilePath $lockFile -Encoding ascii -Force
    $createdLock = $true

    # Checked here rather than with [Parameter(Mandatory)]: in a hidden window a missing mandatory
    # parameter would wait at an invisible prompt forever, leaving Steam stuck on "In-game".
    if (-not $LaunchCode -or -not $GameProcess) {
        throw "Both -LaunchCode and -GameProcess are required, e.g. -LaunchCode WoWF -GameProcess WowB"
    }
    $GameProcess = $GameProcess -replace '\.exe$', ''

    $BnetExe = Resolve-BnetExe -Override $BnetExe
    Write-Log "Battle.net: $BnetExe"
    Write-Log "Launch code: $LaunchCode | Game process: $GameProcess"

    # 0. If the game is already running (started manually, or Play pressed again by mistake),
    #    don't touch Battle.net at all - just pick up tracking the existing session.
    $game = Get-Game
    if ($game) {
        Write-Log "Game process '$GameProcess' already running (PID $($game.Id)) - skipping Battle.net restart"
    } else {
        # 1. Restart Battle.net from this script so the game becomes a child of the Steam launch.
        Stop-Bnet
        Start-Process -FilePath $BnetExe
        $startedBnet = $true

        # 2. Wait for the Battle.net window (up to $BnetMaxWait), then a short settle delay.
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while ($sw.Elapsed.TotalSeconds -lt $BnetMaxWait -and -not (Test-BnetWindow)) {
            Start-Sleep -Milliseconds 500
        }
        if (Test-BnetWindow) { Write-Log ("Battle.net window up after {0:N1}s" -f $sw.Elapsed.TotalSeconds) }
        else                 { Write-Log "No Battle.net window seen after $BnetMaxWait s, continuing anyway" }
        Start-Sleep -Seconds $SettleDelay

        # 3. Send the launch command; resend every $RetryAfter seconds until the game appears
        #    or $StartupWait runs out, rather than giving up after a single resend. A
        #    silently-ignored first attempt now costs one retry interval, not the full timeout.
        Send-Launch
        $deadline  = (Get-Date).AddSeconds($StartupWait)
        $nextRetry = (Get-Date).AddSeconds($RetryAfter)
        while (-not $game -and (Get-Date) -lt $deadline) {
            Start-Sleep -Seconds 1
            $game = Get-Game
            if (-not $game -and (Get-Date) -ge $nextRetry) {
                Write-Log "Game not seen yet, resending launch command"
                Send-Launch
                $nextRetry = (Get-Date).AddSeconds($RetryAfter)
            }
        }
    }

    # 4. Stay alive until the game closes so Steam keeps the in-game status.
    if ($game) {
        Write-Log "Game running (PID $($game.Id)), waiting for exit"
        $game.WaitForExit()
        Write-Log "Game exited"
    } else {
        Write-Log "Game process '$GameProcess' never appeared within $StartupWait s - check -GameProcess / -LaunchCode"
        $exitCode = 2
    }
}
catch {
    Write-Log "ERROR: $($_.Exception.Message)"
    $exitCode = 1
}
finally {
    # Runs on any exit this script controls - normal completion or a caught error. It does
    # NOT run if Steam/Windows force-kills the process outright (e.g. Task Manager "End
    # task", or a hard stop that doesn't let PowerShell run cleanup) - there's no reliable
    # signal-trap equivalent on Windows for that case, so it's a known gap, not a bug.
    if ($startedBnet) { Stop-Bnet }
    if ($createdLock -and (Test-Path -LiteralPath $lockFile)) {
        Remove-Item -LiteralPath $lockFile -Force -ErrorAction SilentlyContinue
    }
    Stop-Transcript | Out-Null
}

exit $exitCode