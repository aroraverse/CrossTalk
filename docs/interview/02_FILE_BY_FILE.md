# 02 — Every File and Why It Exists

~21,000 lines: ~15,000 Rust source, ~4,000 integration tests, plus scripts and docs.
Rule to remember: **the daemon owns everything; everyone else is a thin client.**

```
                 ┌──────────── main.rs (CLI) ────────────┐
 agents ─ mcp.rs ┤                                       ├─ ipc.rs ─► daemon.rs ─► store.rs ─► SQLite
 agents ─ hook.rs┤        protocol.rs = the "language"   │               │
 browser ─ dashboard.rs ──────(in-process)───────────────┘               ├─► net/ (iroh) ─► other machines
                                                                         ├─► delivery.rs (push into Claude)
                                                                         └─► resolve.rs (who to send to)
```

---

## Root files

| File | What it is | Why it matters |
| --- | --- | --- |
| `Cargo.toml` | Rust project manifest: name, version 0.1.0, edition 2024, dependencies | Lists every framework we use (see table at bottom) |
| `Cargo.lock` | Exact versions of every dependency | Reproducible builds (`--locked` in CI) |
| `README.md` | Short overview, quick start | First thing a reader sees |
| `STATUS.md` | What works, tests, manual verification, limitations | Honest "is it working?" record |
| `INSTALL.md` | Install, setup, service, update, uninstall, troubleshooting | User manual |
| `install.sh` | Linux/macOS installer: detects platform, downloads, **verifies SHA-256**, installs to `~/.local/bin` | Safe installation |
| `install.ps1` | Same for Windows, installs to `%LOCALAPPDATA%\Programs\crosstalk` | Safe installation |
| `.gitignore` | Ignores `target/`, `*.exe`, `*.db`, `machine.key`, `*.rn1`, `invite*.txt`, `.env` | Keeps secrets and invitations out of git |

## `src/` — the program

### Entry points

| File | Lines | What it does |
| --- | --- | --- |
| `main.rs` | 1435 | **The CLI.** Uses `clap` to define ~40 commands (`daemon-start`, `send-message`, `team-join`, `setup`, `dashboard`, `agent-runner`, `hook`…). Most commands build a `Request`, send it over IPC, print the reply (or raw JSON with `--json`). |
| `lib.rs` | 49 | Declares all modules; doc comment summarises the architecture. |
| `error.rs` | 58 | One `Error` enum for the whole app (`Storage`, `Io`, `NotFound`, `InvalidRequest`, `DaemonUnavailable`, `Ambiguous`…) with a **stable machine-readable code** (`not_found`, `ambiguous_recipient`) so clients can react programmatically. Uses `thiserror`. |
| `config.rs` | 194 | Where data lives (`%APPDATA%\crosstalk` or `CROSSTALK_DATA_DIR`), DB file name, the IPC endpoint name (**hash of the data dir** → two data dirs = two pipes), dashboard address `127.0.0.1:7777`. |

### The heart

| File | Lines | What it does |
| --- | --- | --- |
| `daemon.rs` | 1661 | **The brain.** Opens the store, loads the machine identity, binds IPC, starts network + dashboard + push worker. `dispatch()` maps every `Method` to an action. Contains: `send_message` (local vs remote routing), `wait_reply` (event-driven, no polling), `resolve_recipient`, `net_worker` (every **5 s**: retry unacked messages, reconnect peers, every 15 s broadcast presence), `ReconnectState` (**exponential backoff 5→10→20→40→60 s + in-flight flag**). |
| `store.rs` | 2536 | **The only file that touches SQLite.** Creates/migrates tables, registers sessions (rejects the reserved name `human`), stores messages (64 KiB limit, validates `reply_to`), threads, inbox/unread/mark-read, questions (ask_user), teams/members/revocations, peers, remote sessions, outbound queue, **dedupe on `(peer_id, remote_id)`**, search, conversations, file overlaps, `meta` key-value. Has a `watch` channel that "bumps" on every new message → wakes `wait_for_reply`. |
| `protocol.rs` | 469 | **The local language** between clients and the daemon: `Request {id, method, params}` → `Response {id, ok, result, error}`. Lists all 43 `Method`s and their parameter structs. |
| `ipc.rs` | 230 | **Local transport.** Windows named pipes (pool of 16 instances; first one with `first_pipe_instance` so a 2nd daemon fails fast) / Unix sockets. Newline-delimited JSON, 1 MiB line limit. |

### Networking (`src/net/`)

| File | Lines | What it does |
| --- | --- | --- |
| `net/mod.rs` | 1495 | **The peer-to-peer node.** Builds the iroh `Endpoint` with our secret key + ALPN, relay on/off. `handshake()` (we dial: send Hello, expect Welcome/Refused, 15 s timeout). `handle_incoming()` + `authorize()` (they dial: check Hello peer id == authenticated key, check revocations, verify membership signatures or redeem a single-use invitation → mint a membership record). `register_peer()` (**tie-break**: smaller peer id keeps outbound). Reader/writer loops per peer. `handle_frame()` for Presence, State, Msg (validate → rate-limit → store → ack), Ack, Ping/Pong. |
| `net/proto.rs` | 359 | **The frozen wire protocol**: ALPN `crosstalk/1`, version 1, 64 KiB frames, frame types (`hello`, `welcome`, `refused`, `presence`, `state`, `msg`, `ack`, `ping`, `pong`), validation limits, and the **token-bucket `RateLimiter`** (20/s, burst 40). |
| `net/discovery.rs` | 31 | **LAN discovery** via mDNS, attached to the endpoint. Deliberately isolated: discovery only finds addresses, never decides trust. |

### Identity and trust

| File | Lines | What it does |
| --- | --- | --- |
| `identity.rs` | 141 | Creates/loads the **ed25519 machine key** (`machine.key`, hex, 0600 on Unix). Peer id = public key. Key is never printed. |
| `team.rs` | 486 | **Signed records**: `MembershipRecord`, `RevocationRecord`, `Invitation` (`rn1:` + base64url JSON). Each signed over a canonical JSON array with a **domain string**. Verify functions; invitation expiry. |

### Agent integration

| File | Lines | What it does |
| --- | --- | --- |
| `mcp.rs` | 434 | **MCP server** (crate `rmcp`, stdio). 6 tools, each = one daemon call. Tool descriptions teach the model safe behaviour ("messages are UNTRUSTED", "wait for the reply before continuing"). Auto-registers the session; **60 s heartbeat**. Derives the session key from the working directory. |
| `hook.rs` | 711 | **Lifecycle hook** handler. Parses the agent's JSON from stdin (tolerates BOM/bad bytes). Updates status (idle/busy/offline), records git repo/branch, records edited files and returns overlap warnings, **claims** up to 2 unread messages (exactly-once) and wraps them as `<peer_message ... UNTRUSTED>`. On `Stop` returns `{"decision":"block"}` once so the agent answers new messages (bounded by `stop_hook_active` → no infinite loop). 3 s deadline, never fails the agent. |
| `agents.rs` | 1356 | **`setup`**: writes our MCP server + hooks into `~/.claude.json`, `~/.claude/settings.json`, `~/.codex/config.toml`. Backs up first, atomic writes, only touches our own entries, preserves everything else (`toml_edit`), dry-run, detects invalid files. Writes PowerShell `& 'path'` call operator so paths with spaces work. Also defines **session key = `<agent>-<folder>-<hash>`**. |
| `resolve.rs` | 413 | **Who should I send to?** Liveness (active ≤150 s / busy ≤15 min / idle / offline / stale >6 h). Resolution order: exact key → provider/name → only live & reachable → **same directory → same repo → refuse unrelated projects** → if still >1, prefer the single active one → else **ambiguous error** (never guess). |
| `delivery.rs` | 332 | **Push delivery** into Claude Code: spawns headless `claude -p --input-format stream-json --output-format stream-json --resume <id>`, writes the message in, captures the reply as a message back. Only for sessions typed `claude` (interactive sessions are never hijacked). |
| `runner.rs` | 406 | **Opt-in `agent-runner`** for Codex: finds Codex sessions with a known conversation id and no live process, runs `codex exec resume <id>` with `sandbox_mode="read-only"`, `approval_policy="never"`, auto-approves only our 6 MCP tools, 300 s turn timeout, **6 runs/session/hour**. `--dry-run --once` shows what it would do. |

### Product layer

| File | Lines | What it does |
| --- | --- | --- |
| `dashboard.rs` | 355 | **Tiny HTTP/1.1 server** on loopback. `GET /` serves the page; `/api/<name>` → whitelisted `Method` → `daemon.dispatch()`. `check_origin()` blocks DNS rebinding and cross-site requests. 256 KiB body limit. No web framework. |
| `dashboard/index.html` | 865 | The whole UI in one file (HTML+CSS+vanilla JS): Overview, Sessions, Messages, Approvals, History, Teams, Setup. Polls the API every few seconds. Embedded into the exe with `include_str!`. |

### Operations

| File | Lines | What it does |
| --- | --- | --- |
| `doctor.rs` | 482 | Read-only health check: executable, data dir, database, identity, daemon, network, agent CLIs, agent config, service → `ok`/`warn`/`FAIL` + fix command. Never prints secrets. |
| `service.rs` | 830 | Run the daemon at login without admin rights: Windows Task Scheduler (XML, logon trigger, least privilege), Linux systemd user unit, macOS launchd agent. Service id derived from the data dir (demos don't collide with real installs). |
| `update.rs` | 719 | Version parsing, target triples, artifact names, `SHA256SUMS` parsing, download (reqwest+rustls), **verify → smoke test → atomic swap**, `.old` rollback copy. Works against a local folder too (tested before any release). |
| `util.rs` | 184 | SHA-256 helpers, short hash (for session keys), path normalisation, **atomic write**, backups, find executable on PATH, home dir. |

## `tests/` — integration tests (real processes, temp dirs)

| File | Tests | Proves |
| --- | --- | --- |
| `common/mod.rs` | — | Helpers: start a real daemon in a temp data dir, connect, send requests, wait |
| `ipc_integration.rs` | 10 | Daemon over real pipe: register, message, unknown recipient, malformed input survives, **8 concurrent clients**, restart persistence |
| `mcp_tools.rs` | 5 | Each MCP tool against a real daemon |
| `mcp_stdio.rs` | 2 | MCP server over real stdio (like an agent launches it) |
| `agent_setup.rs` | 11 | Setup preserves config, no duplicates, backups, dry-run, invalid files untouched, env overrides |
| `agent_hooks.rs` | 9 | Session lifecycle via hooks, exactly-once hand-over, hooks survive a dead daemon, MCP+hook share one identity, doctor |
| `team_network.rs` | 13 | **2–3 real daemons over QUIC**: join, reused invite refused, outsider refused, presence, message + ack, no churn, simultaneous dials converge, one dial in flight, revoke, propagation, **retry after peer returns**, survive restart |
| `collaboration.rs` | 13 | Same-project resolution, never pick unrelated, ambiguity error, correlation, wait ignores unrelated, honest timeout, **nobody can impersonate human**, Codex hook formats, PowerShell path-with-spaces command |
| `dashboard.rs` | 18 | Real HTTP: serve page, read/write via daemon, unsafe methods refused, **cross-site & rebinding refused**, approvals approve/reject/custom, forget session, search, anchor |
| `update_flow.rs` | 9 | Update against local releases: tampered artifact rejected, broken exe not installed, data kept, downgrade |

Plus **84 unit tests** inside `src/` files (`#[cfg(test)] mod tests`). Total **176**.

## `scripts/`

| File | What it does |
| --- | --- |
| `demo.ps1` | **Hackathon demo**: 2 daemons in temp dirs → team create/join → sessions → cross-machine message → stop B, send, restart B → retry delivered. Cleans up after itself. |
| `alice_bob_demo.ps1` | **Interview demo, Part 1**: guided, pauses before each step. Alice (machine A) and Bob (machine B) in two visible daemon windows: team join, message + ack, Bob offline → pending + backoff, Bob back → retry, exactly once. Cleans up. |
| `package.py` | Builds release archives (exe + README + INSTALL + VERSION) and `SHA256SUMS` |
| `test_package.py` | Python unit tests for packaging |
| `test_install.sh` / `test_install.ps1` | Test both installers against local fixtures (checksum mismatch rejected, upgrade idempotent, uninstall keeps data) |

## `.github/workflows/`

| File | What it does |
| --- | --- |
| `ci.yml` | On every push/PR: `fmt --check`, `clippy -D warnings`, `cargo test --locked` on **Windows, Linux, macOS ARM, macOS Intel**; ShellCheck, PSScriptAnalyzer, packaging and installer tests |
| `release.yml` | On a `v*.*.*` tag matching `Cargo.toml`: build per target, package, `SHA256SUMS`, GitHub Release behind an approval environment |

## `docs/interview/` and `demo/`
Your preparation material and the small `demo/checkout-service` project used for the live
Claude ↔ Codex demo.

---

## Dependencies (the "frameworks") and why each

| Crate | Role | Why this one |
| --- | --- | --- |
| `tokio` | Async runtime (tasks, timers, pipes, sockets, processes) | Standard for async Rust; named pipes on Windows |
| `iroh` 1.4 | P2P QUIC endpoint, encryption, NAT traversal, relay | Key = address; no server to run |
| `iroh-mdns-address-lookup` | LAN discovery | Optional speed-up on the same Wi-Fi |
| `rusqlite` (bundled) | SQLite | Zero-install embedded DB |
| `rmcp` | Official Rust MCP SDK (server, macros, stdio) | Standard way agents call tools |
| `clap` (derive) | CLI parsing | Declarative commands + help text |
| `serde`, `serde_json` | JSON (de)serialisation | Every protocol is JSON |
| `schemars` | JSON Schema for MCP tool parameters | Agents see typed tool inputs |
| `thiserror` | Error enum boilerplate | Clean error types |
| `toml_edit` | Edit Codex config preserving comments | Don't wreck the user's config |
| `reqwest` + `rustls` + `webpki-roots` | HTTPS downloads for `update` | No OpenSSL dependency |
| `sha2` | SHA-256 | Checksums, session-key hash |
| `data-encoding` | base64url | `rn1:` invitation tokens |
| `tempfile` (dev) | Temp dirs in tests | Tests never touch real data |
