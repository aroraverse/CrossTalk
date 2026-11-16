# e2e-team: four real agents coordinating through Crosstalk

Claude Code, Codex, Kiro CLI and OpenCode all run in this folder, so they belong to the
same project and find each other by name.

| Step | Script | What it proves |
| --- | --- | --- |
| 1 | `steps\1-register.ps1 -Agent <claude\|codex\|kiro\|opencode>` | each agent's MCP server registers it; `list_team` shows the whole team |
| 2 | `steps\2-claude-ping.ps1`, then `steps\2-answer.ps1 -Agent <codex\|kiro\|opencode>` | Claude resolves each agent by name, sends, and `wait_for_reply` returns each correlated answer |
| 3 | `steps\3-kiro-asks-opencode.ps1`, then `steps\3-opencode-answers.ps1` | two non-Claude agents coordinate directly |

Watch it live: `python -I steps\monitor.py`. Final check: `python -I steps\monitor.py --report`.
Needs the daemon: `crosstalk daemon-start`.
