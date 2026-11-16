# Shared settings for the end-to-end steps. Dot-sourced by every step script.
$ErrorActionPreference = "Continue"
$Project = Split-Path -Parent $PSScriptRoot          # ...\demo\e2e-team
$Ct = Join-Path $env:LOCALAPPDATA "Programs\crosstalk\crosstalk.exe"
Set-Location $Project

# Crosstalk MCP tool names per agent, so only these tools are pre-approved.
$Tools = @("list_team", "resolve_recipient", "send_message", "wait_for_reply", "check_inbox", "ask_user")

function Banner([string]$agent, [string]$step) {
    Write-Host ""
    Write-Host ("=" * 70) -ForegroundColor DarkGray
    Write-Host ("  {0}  |  {1}  |  {2}" -f $agent, $step, (Get-Date -Format HH:mm:ss)) -ForegroundColor Cyan
    Write-Host ("  folder: {0}" -f $Project) -ForegroundColor DarkGray
    Write-Host ("=" * 70) -ForegroundColor DarkGray
}

function Run-Claude([string]$prompt) {
    $allowed = $Tools | ForEach-Object { "mcp__crosstalk__$_" }
    & claude -p $prompt --allowedTools @allowed
}

function Run-Codex([string]$prompt) {
    $approvals = $Tools | ForEach-Object { "-c"; "mcp_servers.crosstalk.tools.$_.approval_mode=approve" }
    & codex exec --skip-git-repo-check @approvals $prompt
}

function Run-Kiro([string]$prompt) {
    # Kiro names MCP tools `@<server>/<tool>`; trust exactly the Crosstalk tools.
    $trusted = ($Tools | ForEach-Object { "@crosstalk/$_" }) -join ","
    & kiro-cli chat --no-interactive --trust-workspace "--trust-tools=$trusted" $prompt
}

function Run-OpenCode([string]$prompt) {
    & opencode run $prompt
}

function Done([string]$agent) {
    Write-Host ""
    Write-Host ("[{0}] step finished at {1} (exit {2})" -f $agent, (Get-Date -Format HH:mm:ss), $LASTEXITCODE) -ForegroundColor Green
}
