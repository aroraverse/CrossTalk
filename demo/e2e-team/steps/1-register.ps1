# Step 1: each agent starts in this folder, registers through its MCP server,
# and discovers the team with list_team.
param([Parameter(Mandatory)][ValidateSet("claude", "codex", "kiro", "opencode")][string]$Agent)
. (Join-Path $PSScriptRoot "_common.ps1")
Banner $Agent "STEP 1: register + discover (list_team)"

$prompt = @"
This is a Crosstalk end-to-end check. Use only the crosstalk MCP tools; do not run shell commands and do not modify any file.
Call the crosstalk list_team tool exactly once.
Then print one line "ME: <your own session key>" (your key starts with "$Agent-e2e-team-"),
and one line "SEES: <session key> <liveness>" for every OTHER session whose working_dir ends with "e2e-team".
Print nothing else.
"@

switch ($Agent) {
    "claude"   { Run-Claude   $prompt }
    "codex"    { Run-Codex    $prompt }
    "kiro"     { Run-Kiro     $prompt }
    "opencode" { Run-OpenCode $prompt }
}
Done $Agent
