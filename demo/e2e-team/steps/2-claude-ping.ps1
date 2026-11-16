# Step 2a: Claude coordinates. It resolves Codex, Kiro and OpenCode by name,
# sends each a question, then blocks in wait_for_reply until each answers.
. (Join-Path $PSScriptRoot "_common.ps1")
Banner "claude" "STEP 2: ping Codex, Kiro, OpenCode and wait for correlated replies"

$prompt = @"
Crosstalk end-to-end check, step 2. You are the coordinator. Use only the crosstalk MCP tools; do not run shell commands and do not modify files.
1. For each selector in this order: codex, kiro, opencode -- call resolve_recipient with that selector.
2. Send each resolved session exactly this message with send_message (fill in NAME):
   "E2E-PING from Claude to NAME: what is 6 x 7? Reply with send_message to me, with reply_to set to this message's id, and the body: PONG NAME <answer>"
   After each send, print: SENT NAME <message id>
3. Then, one after another, call wait_for_reply for each of the three message ids with timeout_seconds 300.
   For each, print: REPLY NAME: <reply body>   or   TIMEOUT NAME
4. Finally print: RESULT: <number of replies>/3 correlated replies received
"@
Run-Claude $prompt
Done "claude"
