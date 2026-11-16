# 01 — Every Concept, From Scratch

Each concept: **one-line meaning → everyday analogy → where it is in Crosstalk → why we chose it.**
Read the bold lines first; they are what you say out loud.

---

## Part A — The problem in plain words

**AI coding agents are like smart interns sitting in separate soundproof rooms.**
Claude Code is in one room, Codex in another. Each is good at its job, but they cannot talk,
cannot see who else is working, and cannot ask each other for a review.

**Crosstalk is the office phone system + reception desk for those interns.**
- The *phone directory* = `list_team` (who is working, on which project, are they free).
- The *phone call* = `send_message`.
- *Waiting on hold for the answer* = `wait_for_reply`.
- *Calling the manager for permission* = `ask_user` (the human approves in the dashboard).
- The *reception desk that never sleeps* = the **daemon**.
- *Two offices in different cities* = two machines connected by **iroh**.

---

## Part B — Core concepts

### 1. Daemon
**A daemon is a program that runs quietly in the background and is always available to serve others.**
- Analogy: the receptionist who sits at the desk all day. You don't "open" her when you need
  her; she's already there.
- Examples you know: Windows services, the print spooler, Docker daemon (`dockerd`).
- In our project: `crosstalk daemon-start` (`src/daemon.rs`). It is the **only** thing that
  touches the database, the local pipe, and the network. Everything else (CLI, MCP server,
  hooks, dashboard) just *asks* the daemon.
- **Why:** agents come and go (you close Claude, the chat restarts), but messages must not be
  lost. Something must live longer than any single agent → a daemon.
- "Single source of truth": one owner of state = no two programs fighting over the database.

### 2. Process, terminal, foreground vs background
**A process is one running program. A terminal is just a window that starts processes.**
- `daemon-start` runs in the **foreground** (it occupies that terminal; Ctrl+C stops it).
- `service install/start` makes it run in the **background** at login (Windows Task Scheduler,
  Linux systemd, macOS launchd) — no terminal needed.

### 3. IPC (Inter-Process Communication)
**IPC is how two programs on the same computer talk to each other without the internet.**
- Analogy: an intercom inside one building — faster and private compared with calling through the
  public phone network.
- Windows: **named pipe** `\\.\pipe\crosstalk-<hash>`. Linux/Mac: **Unix domain socket**
  `daemon.sock` (`src/ipc.rs`).
- Format: **newline-delimited JSON** — one JSON request per line, one JSON reply per line.
- **Why not HTTP on a port for agents?** No network port is opened → nothing on the network
  can reach it; only programs on this computer for this user. The `<hash>` comes from the data
  directory, so two daemons with two data directories get two different pipes (that's how we
  run two daemons on one PC).
- Safety: the first pipe is created with `first_pipe_instance`, so a **second daemon on the
  same data dir fails fast** instead of silently fighting.

### 4. Data directory
**The folder where one daemon keeps everything: database + machine key + logs.**
- Default: `%APPDATA%\crosstalk` (Windows), `~/.local/share/crosstalk` (Linux).
- Override: `CROSSTALK_DATA_DIR` or `--data-dir`.
- **Rule:** one data directory = one daemon = one machine identity. Two data directories on
  one PC = two "virtual machines" as far as the network is concerned.

### 5. SQLite (+ WAL)
**SQLite is a whole database stored in one file, with no separate database server.**
- File: `crosstalk.db`. Library: `rusqlite` with `bundled` (SQLite compiled into our exe,
  so users install nothing).
- **WAL (Write-Ahead Logging)**: writes go to a side log first → readers don't block writers,
  crash-safe.
- **Why SQLite and not Postgres/Redis?** Local-first, zero setup, survives restarts, perfect for
  one process owning the data. Messages, sessions, teams, questions all persist.
- Tables: `sessions`, `messages`, `questions`, `teams`, `team_members`, `used_invites`,
  `revocations`, `remote_sessions`, `peers`, `meta`, `file_activity`.

### 6. MCP (Model Context Protocol)
**MCP is the standard "USB port" through which an AI model calls external tools.**
- Analogy: the AI is a laptop, MCP is USB, our server is a device you plug in. Any MCP-capable
  agent (Claude Code, Codex, Cursor…) can use it.
- Our MCP server (`src/mcp.rs`, crate `rmcp`) runs over **stdio** — the agent launches
  `crosstalk mcp --agent claude` and talks to it via stdin/stdout.
- It is **thin**: every tool = exactly one IPC request to the daemon. No logic, no database.
- 6 tools: `list_team`, `resolve_recipient`, `send_message`, `wait_for_reply`, `check_inbox`, `ask_user`.
- It sends a **heartbeat** every 60 s so the daemon knows the agent is alive.

### 7. Lifecycle hooks
**Hooks are small commands the agent runs automatically at certain moments.**
- Moments: `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Stop`, `SessionEnd`.
- Our hook = `crosstalk hook --agent codex` (`src/hook.rs`). The agent passes JSON on stdin.
- What it does:
  - SessionStart → register the session, tell the model "you are `claude-myproj-ab12cd34`, here are your tools".
  - UserPromptSubmit → status `busy`, hand over new messages.
  - PostToolUse → record which file was edited (overlap warning) + hand over new messages mid-turn.
  - Stop → status `idle`; if messages arrived, **block the stop once** so the agent answers them.
  - SessionEnd → status `offline`.
- **Why hooks + MCP?** MCP = the agent *pulls* (calls a tool). Hooks = we *push* a notice into the
  model's context at natural moments. Together: near-automatic delivery.
- **Hooks never break the agent**: 3-second deadline, every error swallowed, exit 0.
- Limit: a hook only runs when an event happens → an *idle* open window can't be woken.

### 8. Session vs machine (very common interview question)
**A machine is a computer (has one daemon, one key). A session is one agent conversation on it.**
- One machine can host many sessions: `claude-api-1a2b3c4d`, `codex-api-1a2b3c4d`, …
- Session key = `<agent>-<folder>-<hash of the canonical path>`. Hooks and MCP compute it
  independently and get the **same** key → no shared state needed.
- That's why the dashboard can show **2 sessions, 0 peers** on one laptop — correct, not a bug.

### 9. iroh
**iroh is a Rust library that lets two computers connect directly and securely using only a public key as the address.**
- Analogy: instead of dialling a phone number (IP address that changes), you call a person by
  their name (public key) and the phone network figures out how to reach them.
- It gives us: **QUIC** connections, **TLS encryption**, **NAT hole-punching**, and **relay
  fallback**. Made by number0 (n0).
- In code: `src/net/mod.rs` builds an `Endpoint` with our machine's secret key and ALPN
  `crosstalk/1`.
- **Why iroh?** Direct peer-to-peer without our own server; identity = the key, so
  authentication is built in; works behind home routers. Same choice the original Reason
  Network made.

### 10. QUIC
**QUIC is a modern transport protocol (used by HTTP/3) that runs over UDP and has encryption built in.**
- vs TCP: faster connection setup, multiple streams in one connection, survives IP changes,
  always encrypted (TLS 1.3).
- We open **one bidirectional stream per peer** and send JSON frames on it.

### 11. ALPN
**A label both sides agree on when connecting, so they know they're speaking the same protocol.**
- Ours: `crosstalk/1`. Changing the protocol = new label (`/2`), so old and new versions
  never misunderstand each other. "Frozen protocol."

### 12. NAT, hole punching, relay
- **NAT**: your home router hides all your devices behind one public IP. Outsiders can't dial
  you directly.
- **Hole punching**: both sides send packets to each other at the same time so their routers
  "open the door" — then they talk directly.
- **Relay**: if punching fails, traffic goes through an iroh relay server. Still **end-to-end
  encrypted** — the relay just forwards sealed boxes it cannot open.
- Our evidence: the endpoint got a home relay `aps1-1.relay.n0.iroh.link` (Asia-Pacific).
- Switches: `CROSSTALK_RELAY=0` (direct only), `CROSSTALK_DISCOVERY=0` (no mDNS),
  `CROSSTALK_NET=0` (no networking).

### 13. mDNS (LAN discovery)
**mDNS is how devices on the same Wi-Fi find each other without a server (like a printer appearing automatically).**
- `src/net/discovery.rs`, crate `iroh-mdns-address-lookup`.
- **Only an optimisation**: it helps find the address of a peer we already trust. It never
  decides who is trusted. Joining works without it because the invitation carries addresses.

### 14. ed25519, public/private keys, signatures
**A private key signs; the matching public key lets anyone verify the signature. You can't fake a signature without the private key.**
- Each machine creates one ed25519 key → `machine.key` (hex, never printed). Public half = **peer id**.
- We sign: membership records, invitations, revocations — each with a **domain string**
  (`crosstalk/membership/v1`) so a signature for one thing can't be reused for another.
- Signed data is a **canonical JSON array** — unambiguous, avoids delimiter-injection tricks.

### 15. Teams, invitations, membership, revocation
- **Team**: created by one machine = **admin**.
- **Invitation** (`rn1:...` token): base64 JSON with team id, admin id, admin address, nonce,
  expiry, admin signature. **Single-use** (nonce consumed atomically) and **expiring**.
- **Membership record**: admin-signed "peer X belongs to team T". Every connection must show
  one (or a fresh invitation). An encrypted connection alone is **not** enough.
- **Revocation**: admin-signed "peer X is out". Pushed to members; stored; beats a valid
  membership record. **Eventually consistent** (offline members learn later).

### 16. Acknowledgement (ack), retry, dedupe, idempotency
- **Ack**: receiver says "I stored message msg-123". Sender marks it `acked`.
- **Retry**: every 5 s the daemon re-sends anything not acked to connected peers.
- **Dedupe**: retries reuse the **same id**; the receiver has a UNIQUE index on
  `(peer_id, remote_id)` → stored once, acked again. = **exactly-once storage on top of
  at-least-once sending.**
- States: `pending` (peer offline) → `sent` (on the wire) → `acked` (stored remotely) / `failed`.

### 17. Exponential backoff
**After each failure, wait twice as long before trying again (with a cap).**
- Ours: 5 s → 10 s → 20 s → 40 s → 60 s (cap). Success resets it.
- Plus an **in-flight flag**: never two dials to the same peer at once (a dial can take 15 s
  but the worker ticks every 5 s).
- **Why:** don't hammer a dead machine, don't waste battery/CPU, don't spam logs.

### 18. Deterministic tie-break
**If two machines dial each other at the same moment, both pick the same winner by a fixed rule.**
- Rule: the machine with the **smaller peer id keeps its outbound** connection; the larger keeps
  inbound. Both compute the same answer → no "replace, replace, replace" loop.

### 19. Rate limiting (token bucket)
**A bucket with 40 tokens refilled at 20/second; each message costs one token; empty bucket = rejected.**
- Protects a daemon from a flooding peer (`RateLimiter` in `src/net/proto.rs`).

### 20. Correlation (reply_to, thread_id) and wait_for_reply
- Every message has an `id`. A reply carries `reply_to = <original id>` and the same `thread_id`.
- `wait_for_reply` only returns a message whose `reply_to` matches — unrelated messages are
  ignored. It does **not poll**: it subscribes to a `tokio::watch` channel and sleeps until the
  store signals a new message or the timeout (1–900 s) passes.
- On timeout it says so honestly: "Do not assume agreement."

### 21. Untrusted input / prompt injection
**A message from another agent might contain "delete everything" — it must be treated as data, not as an order.**
- Hooks wrap messages in `<peer_message ... (UNTRUSTED)>` tags with a warning.
- The `human` sender name is **reserved**: no session can register as `human`, no peer can send
  as `human`. Only the dashboard/CLI on this machine can.

### 22. Tokio (async)
**Tokio lets one program juggle thousands of waiting tasks (network, pipes, timers) without one thread each.**
- The daemon runs many loops at once: IPC accept loop, network accept loop, per-peer reader and
  writer, 5 s retry ticker, push worker, dashboard server — all `tokio::spawn` tasks.
- `tokio::select!` waits on "whichever happens first" (shutdown, new client, new event, tick).

### 23. Rust
**A systems language that is as fast as C++ but prevents memory bugs and data races at compile time.**
- Why for this: a long-running daemon must not crash or leak; single small binary; great async
  (Tokio) and networking (iroh is Rust-native). One `.exe`, no runtime to install.
- `Arc<Mutex<Store>>` = shared, thread-safe access to one database connection.

### 24. Dashboard (local web UI)
**A web page served by the daemon itself at http://127.0.0.1:7777 — no cloud, no framework.**
- `src/dashboard.rs` is a tiny HTTP/1.1 server on Tokio TCP; UI is one embedded file
  (`src/dashboard/index.html` via `include_str!`).
- Every API call becomes the same `Request` → `daemon.dispatch()` as CLI/MCP. Only a
  **curated whitelist** of methods is exposed.
- Security: loopback `Host` only (blocks **DNS rebinding**), `Origin` must match (blocks
  **cross-site** pages), `Sec-Fetch-Site` checked, JSON content type required (forces CORS
  preflight).

### 25. Push delivery & agent-runner
- **Push delivery** (`src/delivery.rs`): for sessions registered as `claude`, the daemon starts a
  headless `claude -p --input-format stream-json --resume <id>` and writes the message straight
  into that conversation; the reply is stored as a message back.
- **agent-runner** (`src/runner.rs`): opt-in; for Codex sessions with **no running process**,
  runs `codex exec resume <conversation-id>` in a **read-only sandbox**, max **6 runs/hour**,
  marks messages read only after success.

### 26. File-overlap warning
**If two sessions edit the same file in the same repo within 15 minutes, both get a warning.**
- From `PostToolUse` hook → `RecordFileActivity` → `file_overlaps()` query. Advisory, not a lock.

### 27. Anchor
**A flag that says "this always-on machine is the team's mailbox/history keeper."** Stored in
the `meta` table. It does not keep the machine on — the service does.

### 28. CI/CD, checksums, self-update
- **CI** (`.github/workflows/ci.yml`): fmt + clippy + tests on Windows, Linux, macOS ARM, macOS Intel.
- **Release** (`release.yml`): on a `v*.*.*` tag → build, package, `SHA256SUMS`, GitHub release.
- **SHA-256 checksum**: fingerprint of a file; if one byte changes, the fingerprint changes →
  tampered download is rejected.
- **update**: download → verify checksum → smoke-test (`--version`) → atomic swap, old exe kept as `.old`.

### 29. Atomic write & backup
**Write to a temp file, then rename over the original — the file is never half-written.**
`setup` backs up `~/.claude.json`, `~/.claude/settings.json`, `~/.codex/config.toml` first, and
uses `toml_edit` so comments/formatting in the user's Codex config are preserved.

---

## Part C — 10 words to drop naturally in the interview

local-first · single source of truth · thin clients · at-least-once + dedupe = exactly-once ·
exponential backoff · deterministic tie-break · signed membership · eventually consistent
revocation · untrusted peer input · human-in-the-loop
