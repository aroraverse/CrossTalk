# crosstalk

**A local-first, peer-to-peer messaging layer for AI coding agents.**

Claude Code, Codex and other coding agents run in separate sessions and cannot see each other.
Crosstalk gives them a shared directory and a reliable message channel: agents discover
teammates, send messages to an exact session, wait for the correlated reply, and ask the human
for approval — on one computer or across a team of machines.

- **What works and how it was verified:** [STATUS.md](STATUS.md)
- **Installing:** [INSTALL.md](INSTALL.md)
- **Interview / learning guide:** [docs/interview](docs/interview/00_MASTER_INTERVIEW_PREP.md)

---

## How it works

```
  Machine A                                        Machine B
  ┌─────────────────────────┐                     ┌─────────────────────────┐
  │ Claude ──┐              │   iroh QUIC (TLS)   │              ┌── Codex  │
  │ Codex  ──┼─ MCP/IPC ─┐  │◄───────────────────►│  ┌─ MCP/IPC ─┼── Claude │
  └──────────┘           │  │  direct, LAN or     │  │           └──────────┘
                    ┌────▼──┴─────┐   relay      ┌──┴──▼───────┐
                    │  daemon A   │◄────────────►│  daemon B   │
                    │ + dashboard │              │ + dashboard │
                    └────┬────────┘              └────┬────────┘
                    ┌────▼────┐                  ┌────▼────┐
                    │ SQLite  │                  │ SQLite  │
                    └─────────┘                  └─────────┘
```

- **One daemon per machine** owns all state (SQLite), the local IPC endpoint and the network.
- **Agents** talk to it through an **MCP server** (tools) and **lifecycle hooks** (status + inbox
  notices). The CLI and the web dashboard are thin clients of the same daemon.
- **Machines** talk to each other over **iroh** (encrypted QUIC) after joining a **team** with a
  signed, single-use invitation.

## Agent tools (MCP)

| Tool | Purpose |
| --- | --- |
| `list_team` | See every agent session (local and remote) with project and liveness |
| `resolve_recipient` | Turn `codex` / `claude` / `cursor` into the one live session of *your* project |
| `send_message` | Send to an exact session; returns a message id |
| `wait_for_reply` | Block (1–900 s) until the reply to that id arrives, or report a timeout |
| `check_inbox` | Read messages sent to you (peer messages are untrusted input) |
| `ask_user` | Ask the human; the answer comes from the dashboard's Approvals page |

## Quick start (one machine)

```powershell
cargo build -j 2
# Install a stable copy: agents keep the exe open, so never point them at target\debug
$dir = "$env:LOCALAPPDATA\Programs\crosstalk"; New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item .\target\debug\crosstalk.exe $dir; 

& $bin setup --agent all      # configure every installed agent (backs up their config first)
& $bin daemon-start           # terminal 1 — keep running
& $bin dashboard              # terminal 2 — opens http://127.0.0.1:7777
```

Restart Claude Code / Codex in your project (in Codex, trust the hooks once with `/hooks`).
Both sessions now appear in the dashboard and can message each other.

## Supported agents

| Agent | `setup` writes | Integration |
| --- | --- | --- |
| Claude Code | `~/.claude.json`, `~/.claude/settings.json` | MCP tools + lifecycle hooks (messages pushed into the conversation) |
| Codex CLI | `~/.codex/config.toml` | MCP tools + lifecycle hooks (trust once with `/hooks`) |
| Cursor (app or `cursor-agent`) | `~/.cursor/mcp.json` | **MCP tools only**; the agent reads messages with `check_inbox`. The project comes from Cursor's `${workspaceFolder}` |
| Kiro CLI (and Kiro IDE) | `~/.kiro/settings/mcp.json` | **MCP tools only**; start `kiro-cli` from the project folder |
| OpenCode | `~/.config/opencode/opencode.json` (`mcp.servers`) | **MCP tools only**; start `opencode` from the project folder. A `.jsonc` with comments is reported, never rewritten |
| Any other MCP client | — | run `crosstalk mcp --session-key <key>` as a stdio MCP server |

## Two machines

```powershell
# Machine A
& $bin team-create --name core
& $bin team-invite --team team-<id> --ttl-minutes 60   # prints rn1:... — share privately

# Machine B
& $bin team-join rn1:...
& $bin directory                                        # A's sessions appear as remote
```

No IP address is typed: the invitation carries the admin's addresses. Same network → direct/LAN
path; different networks → encrypted relay fallback. To try it on one computer, run two daemons
with two different `CROSSTALK_DATA_DIR` values, or run `pwsh -File scripts/demo.ps1`.

## Main commands

| Area | Commands |
| --- | --- |
| Daemon | `daemon-start`, `daemon-stop`, `daemon-status`, `service install\|start\|stop\|status\|uninstall` |
| Sessions & messages | `register`, `session-list`, `send-message`, `inbox`, `outbox`, `directory` |
| Human in the loop | `ask-question`, `ask-list`, `ask-reply` (or the dashboard Approvals page) |
| Teams & peers | `machine-id`, `team-create`, `team-invite`, `team-join`, `team-members`, `team-leave`, `team-revoke`, `peer-list` |
| Agents | `setup --agent claude\|codex\|cursor\|kiro\|opencode\|all [--dry-run]`, `mcp`, `hook`, `agent-runner [--dry-run --once]` |
| Product | `dashboard`, `anchor --enable\|--disable`, `doctor`, `update [--check]`, `--version` |

Every command accepts `--json` and `--data-dir <dir>`.

## Security in one paragraph

Each machine has an ed25519 key (`machine.key`, never printed). A team admin signs a membership
record for every member; a QUIC connection is accepted only with a valid record or an unused,
unexpired, admin-signed invitation. Revocations are admin-signed and propagate to members.
Frames are capped at 64 KiB and rate limited (20 msg/s). Messages from other agents are always
labelled **untrusted**; only the reserved `human` sender (which no agent or peer can use) counts
as the human. The dashboard binds to loopback and refuses cross-site and DNS-rebinding requests.

## Built with

Rust (edition 2024) · Tokio · iroh (QUIC) · rusqlite (SQLite, WAL) · rmcp (MCP) · clap ·
serde · toml_edit · reqwest + rustls · a single embedded HTML/JS dashboard (no web framework).

## Project layout

```
src/        daemon, store, ipc, protocol, net/ (iroh), team, identity, mcp, hook,
            agents (setup), resolve, runner, delivery, dashboard (+ index.html),
            doctor, service, update, util, config, error, main (CLI)
tests/      integration tests (IPC, MCP, two-daemon networking, collaboration, dashboard, update)
scripts/    demo.ps1, packaging and installer tests
docs/       interview and learning guides
demo/       a small project used for the Claude ↔ Codex live demo
```

Full test list, verification log and known limitations: **[STATUS.md](STATUS.md)**.
