# Crosstalk — Status and Verification

This file records **what works, how it was verified, and what is known not to work**.
The [README](README.md) stays short; every "is this working?" detail lives here.

Latest verified run: **183 automated tests passing, 0 failing**, plus `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` clean.

---

## 1. Milestones

| Milestone | Scope | State |
| --- | --- | --- |
| **M0** | Local foundation: daemon, SQLite, local IPC, session registry, CLI, MCP tools | Done |
| **M1** | Encrypted peer-to-peer team networking on iroh: machine identity, signed membership, invitations, remote messaging with acks and retries, presence sync, LAN discovery, relay | Done, protocol frozen |
| **M2** | Agent integrations: `setup` for Claude Code and Codex (MCP + lifecycle hooks), `doctor`, background `service` | Done |
| **M3** | CI and release workflows, versioned packages with SHA-256 checksums, installers, `--version`, `update` | Done, **no release published yet** |
| **M4** | Local dashboard, messaging UI, approvals, history search, onboarding, file-overlap warnings, anchor role | Done |
| **Collaboration** | `resolve_recipient`, `wait_for_reply`, reply/thread correlation, exactly-once hook hand-over, human-sender protection, opt-in Codex `agent-runner` | Done |

---

## 2. Feature status

| Requirement | State |
| --- | --- |
| Persistent machine identity (ed25519, generated once, never printed) | Working |
| Team creation, single-use signed invitations, joining | Working |
| Membership verification on every connection; unauthorised peers refused | Working |
| Encrypted peer transport with a frozen versioned protocol (iroh/QUIC, ALPN `crosstalk/1`) | Working |
| Remote messaging with persistence, acknowledgement, retry, dedupe | Working |
| Presence sync of local sessions to team peers | Working |
| Admin-signed revocation; revoked peers are refused | Working, eventually consistent |
| Connection stability: single dial per peer, exponential backoff, deterministic duplicate resolution | Working |
| M0 local messaging, CLI and MCP behaviour preserved | Working |
| LAN discovery (mDNS) | Working; endpoint-attached lookup |
| Relay connectivity | Working; endpoint acquires a home relay and publishes a relay address |
| Claude Code setup, MCP and lifecycle hooks | Working; on Windows hooks use the exec form (no shell), verified with real Claude Code 2.1.318 |
| Codex setup and MCP | Working; Codex loads the config and starts the server |
| Cursor setup (MCP server in `~/.cursor/mcp.json`, project via `${workspaceFolder}`), no hooks | Working; MCP path verified end to end against a daemon; not yet verified inside the Cursor app |
| Kiro CLI setup (MCP server in `~/.kiro/settings/mcp.json`), no hooks | Working; format confirmed with `kiro-cli mcp list/status`; the server's start folder inside a live Kiro chat not yet verified |
| OpenCode setup (`mcp.servers` in `opencode.json`), no hooks | Working; verified with OpenCode 1.18.32: server connects and registers the project's session |
| Codex lifecycle hooks | Working; verified with Codex 0.166 from a path containing spaces and an en dash |
| Recipient resolution (`resolve_recipient`) — same project only, explicit ambiguity errors | Working |
| Correlated replies (`reply_to`, `thread_id`) and bounded `wait_for_reply` (1–900 s) | Working |
| Exactly-once hand-over of messages to agents (`ClaimMessages`) | Working |
| Reserved `human` sender cannot be impersonated by agents or peers | Working |
| Opt-in Codex `agent-runner` (read-only sandbox, 6 runs/session/hour) | Working |
| `doctor` | Working |
| Background service | Working; Windows Task Scheduler verified; systemd and launchd definitions unit-tested only |
| Dashboard (Overview, Sessions, Messages, Approvals, History, Teams, Setup) | Working; loopback-only |
| File-overlap warnings | Working; advisory only |
| Anchor role | Working (a flag; does not keep a machine powered on) |
| CI and release workflows | Working; linted with actionlint |
| Installers, checksums, `update` | Working; verified against local release fixtures |

---

## 3. Automated tests (183)

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -j 2
```

All tests run in temporary directories and never touch real user data.

| Area | Where | Tests |
| --- | --- | --- |
| Machine identity stable across restarts; signatures | `src/identity.rs` | 3 |
| Membership records, forged/tampered records, invitation tokens, revocation | `src/team.rs` | 10 |
| Session registry, messaging, read state, dedupe, retry state, rosters, search, overlaps | `src/store.rs` | 17 |
| Recipient resolution and liveness | `src/resolve.rs` | 6 |
| Reconnect backoff schedule, in-flight guard, success reset, agent replies | `src/daemon.rs` | 6 |
| Protocol framing, version rejection, oversized frames, rate limiter | `src/net/` | 9 |
| Setup, service, update, util, hook, runner, dashboard, config, CLI units | `src/*.rs` | 34 |
| Local IPC, invalid input, concurrent clients, restart persistence | `tests/ipc_integration.rs` | 10 |
| MCP tools and MCP over real stdio | `tests/mcp_tools.rs`, `tests/mcp_stdio.rs` | 7 |
| Setup against isolated configuration fixtures, old hook formats replaced, Cursor / Kiro / OpenCode MCP setup | `tests/agent_setup.rs` | 18 |
| Hooks, shared MCP/hook identity, doctor | `tests/agent_hooks.rs` | 9 |
| Two/three real daemons over QUIC: invite/join, refusal, presence, ack, retry, revocation, connection stability | `tests/team_network.rs` | 13 |
| Collaboration: resolution, correlation, wait/timeout, exactly-once claim, human protection, generated Claude and Codex hook commands run as the agents run them | `tests/collaboration.rs` | 14 |
| Dashboard over real loopback HTTP, incl. cross-site / DNS-rebinding refusal | `tests/dashboard.rs` | 18 |
| Update checks and self-update against local releases, tampered artifacts | `tests/update_flow.rs` | 9 |

The `team_network` tests run real daemons with separate data directories and separate iroh
identities, connecting over actual QUIC with relay and discovery disabled.

---

## 4. Verified manually

Performed on one Windows 11 machine with two independent daemons (`%TEMP%\rn-m1-a`,
`%TEMP%\rn-m1-b`), relay and discovery disabled:

1. `team-create` on A, `team-invite`, `team-join` on B — joined, roster of 2.
2. `peer-list` on both — connected.
3. `directory` on A — B's session appears as `local: no` with B's machine id.
4. `send-message` A → B — `delivery: sent`, then `outbox` shows `acked`.
5. `inbox` on B — the message body, sender session, unread state.
6. Stopped B, sent from A — `delivery: pending`; restarted B — automatic reconnect, retry,
   `acked`, and B's inbox contains the message exactly once.
7. Two machines left connected for 12 s — `replacements` stayed `0`.
8. Both daemons restarted and dialled each other **simultaneously** — one redundant connection
   resolved by the tie-break, no further replacement, one connected peer each.
9. A restarted while B held A's old address — zero overlapping handshake timeouts after the fix.
10. A running against an offline peer for 50 s — 2 dial attempts (5 s, then 10 s) instead of ~10.
11. `team-revoke` on the admin: member removed, connection closed, reconnect refused with `revoked`.

Relay: with relay enabled (default) the endpoint acquired a home relay
(`https://aps1-1.relay.n0.iroh.link./`) and published a relay address plus a public IP in its
`EndpointAddr`, which is what the invitation carries.

Agents:

- Real Claude Code ↔ Codex proposal → review → human approval → acknowledgement workflow.
- Real Codex 0.166 hooks from a path with spaces and an en dash: `SessionStart`,
  `UserPromptSubmit`, `Stop`, `SessionEnd` logged as successful.
- Dashboard responsive checks at 420, 700, 1024 and 1440 px, no horizontal overflow, no JS errors.

---

## 5. Deployment notes (networking across machines)

- **Same machine, one daemon.** Claude and Codex share one daemon → local sessions, `0` peers.
  This is correct, not a bug.
- **Two daemons, one machine.** Use two different data directories → two machine identities →
  real QUIC between them. This is how the networking is tested.
- **Two machines, same LAN.** mDNS speeds up discovery; the direct path is used.
- **Two machines, different networks.** Relay is on by default. The endpoint publishes a relay
  address, which the invitation carries, so peers behind NAT can still connect; iroh tries to
  upgrade to a direct (hole-punched) path.
- **mDNS is an optimisation only.** Joining works without it because the invitation carries the
  admin's addresses.

---

## 6. Known limitations

1. **Idle interactive Codex window** cannot be woken by a hook. It receives messages at its next
   prompt / tool call / turn end. The opt-in runner resumes only sessions with *no* running process.
2. **Runner approvals:** the runner auto-approves Crosstalk's own MCP tools during unattended
   turns (read-only sandbox). Review `agent-runner --dry-run --once` before enabling it.
3. **Codex hook trust:** the user must trust the hooks once through Codex's `/hooks` (again
   whenever the hook command changes).
   **Rebuilding while agents run:** on Windows, running agents keep `crosstalk mcp` open, which
   locks the executable, so `cargo build` cannot replace it. Point the agents at an installed copy
   (`%LOCALAPPDATA%\Programs\crosstalk`) and run `setup` from there; never run `setup` from a
   stale build, because it writes that build's hook format.
4. **Revocation is eventually consistent:** an offline member keeps stale state until it reconnects.
   A revoked peer is not notified, and there is no un-revoke.
5. **Single admin** per team; no admin transfer or multi-admin quorum.
6. **Presence** is a full snapshot per peer, not a delta.
7. **One team per invitation;** invitations are single-use.
8. **Cross-host networking:** local two-daemon tests do not prove every NAT, relay-only or
   cross-host mDNS scenario.
9. **Cross-platform services:** Windows verified; systemd/launchd are unit-tested definitions only.
10. **Release distribution:** an earlier Windows release build was flagged by Defender. Use a
    verified debug build; do not bypass antivirus.
11. **Dashboard is loopback-only** (`127.0.0.1:7777`). It is not a hosted web app.
12. **File-overlap warnings are advisory** — not a lock or merge system.
13. **No file transfer;** messages are text up to 64 KiB.

---

## 7. Wire protocol (frozen for M1)

- ALPN `crosstalk/1`, protocol version `1`
- One JSON object per line, maximum 64 KiB, envelope `{"v":1,"body":{"type":"<frame>",...}}`

| Frame | Direction | Content |
| --- | --- | --- |
| `hello` | both | peer id, machine name, current address, roster, invitations |
| `welcome` | both | roster for the shared teams, machine name, address |
| `refused` | acceptor | `code` + `reason` before closing |
| `presence` | both | sessions hosted by the sender |
| `state` | both | membership roster and revocations |
| `msg` | sender | message id, sender/recipient session, body, created_at, reply_to, thread_id |
| `ack` | receiver | message id, accepted, error |
| `ping` / `pong` | both | keep-alive |

Hardening: 64 KiB frame limit, per-connection token bucket (20 msg/s, burst 40), `(peer, message id)`
uniqueness, version check on every frame, `Hello` peer id and address must match the
authenticated QUIC connection.

## 8. Storage

M0 tables: `sessions`, `messages`, `questions`. M1 adds `teams`, `team_members`, `used_invites`,
`revocations`, `remote_sessions`, `peers`, `meta`, plus `origin`, `peer_id`, `remote_id`,
`delivery_state` on `messages`. Collaboration adds `reply_to`, `thread_id`. M4 adds
`file_activity`. Existing databases are migrated in place.
