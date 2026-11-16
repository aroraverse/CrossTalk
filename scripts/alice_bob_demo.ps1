<#
.SYNOPSIS
    Guided live demo: two dummy agents, Alice and Bob, on two independent
    daemons ("machines"). Shows message + acknowledgement, what happens when
    Bob goes offline (queue + exponential backoff), and automatic retry with
    exactly-once delivery when Bob comes back.

.DESCRIPTION
    pwsh -File scripts/alice_bob_demo.ps1

    Opens two windows, "Machine A - Alice" and "Machine B - Bob", each running
    its own daemon with its own data directory and identity. This window
    drives the demo and pauses before every step so you can talk.

    Everything lives in temporary directories; your real installation and
    identity are never touched. Relay and LAN discovery are off, so the demo
    does not depend on Wi-Fi or the internet.

.PARAMETER Bin
    crosstalk executable. Defaults to the installed copy, then target\debug.
.PARAMETER NoPause
    Run straight through without waiting for Enter (for rehearsal checks).
.PARAMETER Hidden
    Run the daemon windows hidden (for automated checks).
#>
param(
    [string]$Bin = "",
    [switch]$NoPause,
    [switch]$Hidden
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Always run in a child process. This script switches CROSSTALK_DATA_DIR
# between the two demo machines; run in-process (`.\alice_bob_demo.ps1`) that
# would leak into the caller's terminal, and a later `setup` there would pin
# the agents to a temporary demo daemon.
if (-not $env:RN_ALICE_BOB_CHILD) {
    $shellExe = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
    $childArgs = @("-NoProfile", "-File", $PSCommandPath)
    if ($Bin) { $childArgs += @("-Bin", $Bin) }
    if ($NoPause) { $childArgs += "-NoPause" }
    if ($Hidden) { $childArgs += "-Hidden" }
    $env:RN_ALICE_BOB_CHILD = "1"
    try { & $shellExe @childArgs } finally { Remove-Item Env:RN_ALICE_BOB_CHILD -ErrorAction SilentlyContinue }
    exit $LASTEXITCODE
}

# ── binary ──────────────────────────────────────────────────────────────────
if (-not $Bin) {
    $installed = Join-Path $env:LOCALAPPDATA "Programs\crosstalk\crosstalk.exe"
    $debug = Join-Path (Split-Path -Parent $PSScriptRoot) "target\debug\crosstalk.exe"
    $Bin = if (Test-Path $installed) { $installed } else { $debug }
}
if (-not (Test-Path $Bin)) { Write-Host "crosstalk.exe not found: $Bin" -ForegroundColor Red; exit 1 }
$Shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }

# ── two machines on one laptop ─────────────────────────────────────────────
$Root = Join-Path $env:TEMP "rn-alice-bob"
$DirA = Join-Path $Root "machine-a"
$DirB = Join-Path $Root "machine-b"
$LogA = Join-Path $Root "machine-a.log"
$LogB = Join-Path $Root "machine-b.log"

$env:CROSSTALK_RELAY = "0"       # direct connections only
$env:CROSSTALK_DISCOVERY = "0"   # no mDNS: deterministic

function A { $env:CROSSTALK_DATA_DIR = $DirA; & $Bin @args }
function B { $env:CROSSTALK_DATA_DIR = $DirB; & $Bin @args }

function Step([string]$title, [string]$say) {
    Write-Host ""
    Write-Host ("=" * 72) -ForegroundColor DarkGray
    Write-Host "  $title" -ForegroundColor Cyan
    if ($say) { Write-Host "  Say: $say" -ForegroundColor DarkYellow }
    Write-Host ("=" * 72) -ForegroundColor DarkGray
    if (-not $NoPause) { [void](Read-Host "  [Enter] to run") }
}

function Show([string]$label, [scriptblock]$block) {
    Write-Host "> $label" -ForegroundColor Green
    & $block | Out-Host
}

function Start-Machine([string]$title, [string]$dir, [string]$log, [string]$port) {
    $cmd = "`$Host.UI.RawUI.WindowTitle = '$title'; " +
           "`$env:CROSSTALK_DATA_DIR = '$dir'; " +
           "`$env:CROSSTALK_RELAY = '0'; `$env:CROSSTALK_DISCOVERY = '0'; " +
           "`$env:CROSSTALK_DASHBOARD_ADDR = '127.0.0.1:$port'; " +
           "Write-Host '$title  (dashboard http://127.0.0.1:$port)' -ForegroundColor Cyan; " +
           "& '$Bin' daemon-start 2>&1 | ForEach-Object { `"`$_`" } | Tee-Object -FilePath '$log' -Append"
    # Visible windows stay open after the daemon stops so the log can be read;
    # hidden ones (automated checks) exit with their daemon.
    $shellArgs = if ($Hidden) { @("-NoProfile", "-Command", $cmd) } else { @("-NoProfile", "-NoExit", "-Command", $cmd) }
    $style = if ($Hidden) { "Hidden" } else { "Normal" }
    Start-Process $Shell -ArgumentList $shellArgs -WindowStyle $style | Out-Null
}

function Wait-Until([string]$what, [int]$seconds, [scriptblock]$condition) {
    Write-Host "  waiting: $what" -NoNewline -ForegroundColor DarkGray
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.Elapsed.TotalSeconds -lt $seconds) {
        if (& $condition) { Write-Host (" ({0:N0}s)" -f $watch.Elapsed.TotalSeconds) -ForegroundColor DarkGray; return $true }
        Start-Sleep -Milliseconds 1000
        Write-Host "." -NoNewline -ForegroundColor DarkGray
    }
    Write-Host " (gave up after ${seconds}s)" -ForegroundColor Yellow
    return $false
}

function Connected([scriptblock]$machine) {
    try { ((& $machine --json machine-id | ConvertFrom-Json).connected).Count -gt 0 } catch { $false }
}

function State-Of([string]$id) {
    $line = A outbox | Select-String ([regex]::Escape($id.Substring(0, [Math]::Min(18, $id.Length))))
    if ($line) { ($line.ToString() -split '\s+')[2] } else { "" }
}

# ── fresh start ─────────────────────────────────────────────────────────────
foreach ($d in $DirA, $DirB) { $env:CROSSTALK_DATA_DIR = $d; & $Bin daemon-stop *> $null }
Start-Sleep -Milliseconds 500
Remove-Item -Recurse -Force $Root -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $Root | Out-Null

Write-Host "`nCrosstalk - Alice & Bob demo   ($Bin)" -ForegroundColor Cyan

# ── 1. two machines ─────────────────────────────────────────────────────────
Step "1. Start two independent machines (two daemons, two identities)" `
     "Each window is a separate machine: its own daemon, database and cryptographic identity."
Start-Machine "Machine A - Alice" $DirA $LogA "7781"
Start-Machine "Machine B - Bob"   $DirB $LogB "7782"
[void](Wait-Until "both daemons ready" 30 {
    A daemon-status *> $null; $a = $LASTEXITCODE
    B daemon-status *> $null; ($a -eq 0) -and ($LASTEXITCODE -eq 0)
})
Show "machine-id on A" { A machine-id }
Show "machine-id on B" { B machine-id }

# ── 2. team + agents ────────────────────────────────────────────────────────
Step "2. Form a team with a signed invitation and register the two agents" `
     "A creates a team and signs a single-use invitation; B joins with it. No IP address is typed."
$team = A --json team-create --name demo | ConvertFrom-Json
Write-Host "> team-create on A -> $($team.team_id)" -ForegroundColor Green
$token = (A team-invite --team $team.team_id --ttl-minutes 30 | Select-String '^rn1:').ToString()
Write-Host ("> team-invite on A -> {0}... ({1} chars, single-use, expires in 30 min)" -f $token.Substring(0, 16), $token.Length) -ForegroundColor Green
Show "team-join on B" { B team-join $token }
Show "register agent Alice on A" { A register --name alice --session-key alice --agent-type claude }
Show "register agent Bob on B"   { B register --name bob --session-key bob --agent-type codex }
[void](Wait-Until "Bob's presence reaches A" 30 { (A directory | Select-String '^bob\s') -ne $null })
Show "directory on A (Bob is remote: LOCAL = no)" { A directory }

# ── 3. message + ack ────────────────────────────────────────────────────────
Step "3. Alice sends Bob a message; Bob's machine acknowledges it" `
     "Stored first, sent over encrypted QUIC, and acknowledged only after Bob's machine stored it."
$sent = A --json send-message --from alice --to bob "Hi Bob, please review the storage layer." | ConvertFrom-Json
Write-Host "> send-message alice -> bob : $($sent.id)  delivery: $($sent.delivery_state)" -ForegroundColor Green
[void](Wait-Until "acknowledgement" 15 { (State-Of $sent.id) -eq "acked" })
Show "outbox on A" { A outbox }
Show "inbox on B" { B inbox --session bob }

# ── 4. Bob goes offline ─────────────────────────────────────────────────────
Step "4. Bob's machine goes offline; Alice sends anyway" `
     "Nothing is lost: the message is queued as pending, and A retries with exponential backoff."
Show "stop Bob's daemon" { B daemon-stop }
[void](Wait-Until "A notices Bob is gone (QUIC idle timeout, ~30s)" 60 { -not (Connected ${function:A}) })
$queued = A --json send-message --from alice --to bob "Sent while you were offline." | ConvertFrom-Json
Write-Host "> send-message alice -> bob : $($queued.id)  delivery: $($queued.delivery_state)" -ForegroundColor Green
Show "outbox on A" { A outbox }
Write-Host "  Watch the 'Machine A - Alice' window: reconnect attempts back off 5s -> 10s -> 20s ..." -ForegroundColor DarkYellow
[void](Wait-Until "two backoff attempts in A's log" 60 { @(Select-String -Path $LogA -Pattern 'next attempt in' -ErrorAction SilentlyContinue).Count -ge 2 })
Show "A's log" { Select-String -Path $LogA -Pattern 'next attempt in|disconnected' | ForEach-Object { $_.Line } | Select-Object -Last 4 }

# ── 5. Bob returns ──────────────────────────────────────────────────────────
Step "5. Bob comes back; the queued message is delivered exactly once" `
     "Reconnect, retry, acknowledge. Retries reuse the same id, so Bob stores it once."
Start-Machine "Machine B - Bob (restarted)" $DirB $LogB "7782"
[void](Wait-Until "reconnect + retry + ack" 90 { (State-Of $queued.id) -eq "acked" })
Show "outbox on A" { A outbox }
Show "inbox on B (each message exactly once)" { B inbox --session bob }
Show "connection health on A" { A --json machine-id | ConvertFrom-Json | Select-Object connected, replacements, reconnecting | Format-List }

# ── cleanup ─────────────────────────────────────────────────────────────────
Step "Done. Stop both machines and delete the temporary data" ""
foreach ($d in $DirA, $DirB) { $env:CROSSTALK_DATA_DIR = $d; & $Bin daemon-stop *> $null }
Start-Sleep -Seconds 1
Remove-Item -Recurse -Force $Root -ErrorAction SilentlyContinue
Write-Host "Cleaned up. (The two daemon windows can be closed.)" -ForegroundColor Cyan
