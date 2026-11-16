# Run one showcase agent headless in refund-service, show its output live and
# log every line with a millisecond timestamp (the video replays these logs).
param(
    [Parameter(Mandatory)][ValidateSet("claude", "codex", "kiro", "opencode")][string]$Agent,
    [Parameter(Mandatory)][string]$Prompt,          # file name in ..\prompts
    [string]$DataDir = (Join-Path $env:TEMP "ct-showcase")
)
$ErrorActionPreference = "Continue"
$env:CROSSTALK_DATA_DIR = $DataDir
$here = $PSScriptRoot
$work = Join-Path (Split-Path $here -Parent) "refund-service"
$text = (Get-Content (Join-Path (Split-Path $here -Parent) "prompts\$Prompt") -Raw)
$logs = Join-Path $here "logs"; New-Item -ItemType Directory -Force $logs | Out-Null
$log = Join-Path $logs ("{0}-{1}.log" -f $Agent, [IO.Path]::GetFileNameWithoutExtension($Prompt))
Set-Location $work
$tools = @("list_team", "resolve_recipient", "send_message", "wait_for_reply", "check_inbox", "ask_user")

function Stamp { process { $line = "$_"; "{0}`t{1}" -f [DateTimeOffset]::Now.ToUnixTimeMilliseconds(), $line | Add-Content -Encoding utf8 $log; $line } }

"{0}`tSTART {1} {2}" -f [DateTimeOffset]::Now.ToUnixTimeMilliseconds(), $Agent, $Prompt | Set-Content -Encoding utf8 $log
Write-Host ("== {0} :: {1} :: {2}" -f $Agent, $Prompt, (Get-Date -Format HH:mm:ss)) -ForegroundColor Cyan
switch ($Agent) {
    "claude" {
        $allowed = @("Read", "Edit", "Write", "Glob", "Grep", "Bash(python -m unittest:*)", "Bash(python:*)") + ($tools | ForEach-Object { "mcp__crosstalk__$_" })
        & claude -p $text --output-format stream-json --verbose --allowedTools @allowed 2>&1 | Stamp
    }
    "codex" {
        $approvals = $tools | ForEach-Object { "-c"; "mcp_servers.crosstalk.tools.$_.approval_mode=approve" }
        # Codex starts MCP servers with a clean environment: pass the data dir explicitly.
        $dd = $DataDir.Replace('\', '/')
        & codex exec --json --skip-git-repo-check -c "mcp_servers.crosstalk.env.CROSSTALK_DATA_DIR=`"$dd`"" @approvals $text 2>&1 | Stamp
    }
    "kiro" { & kiro-cli chat --no-interactive --trust-workspace --trust-all-tools $text 2>&1 | Stamp }
    "opencode" { & opencode run $text 2>&1 | Stamp }
}
"{0}`tEND exit={1}" -f [DateTimeOffset]::Now.ToUnixTimeMilliseconds(), $LASTEXITCODE | Add-Content -Encoding utf8 $log
Write-Host ("== {0} finished {1}" -f $Agent, (Get-Date -Format HH:mm:ss)) -ForegroundColor Green
