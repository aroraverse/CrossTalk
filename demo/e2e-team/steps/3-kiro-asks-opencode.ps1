# Step 3a: two non-Claude agents coordinate directly. Kiro asks OpenCode a
# question and waits for the correlated reply.
. (Join-Path $PSScriptRoot "_common.ps1")
Banner "kiro" "STEP 3: Kiro asks OpenCode directly and waits"

$prompt = @"
Crosstalk end-to-end check, step 3. Use only the crosstalk MCP tools; do not run shell commands and do not modify files.
1. Call resolve_recipient with selector opencode.
2. send_message to that session: "E2E-RELAY from Kiro: name one prime number greater than 10. Reply with send_message to me, with reply_to set to this message's id, and the body: RELAY-ACK opencode <prime>"
   Print: SENT <message id>
3. Call wait_for_reply with that message id and timeout_seconds 300.
   Print: REPLY: <reply body>   or   TIMEOUT
"@
Run-Kiro $prompt
Done "kiro"
