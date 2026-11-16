# 03 — Feature Showcase: "Show, Don't Tell"

Each feature below proves deep knowledge. For every one: **what to say (1 line)** → **how to show it live** → **where in code** → **the follow-up they will ask**.

All commands below were **run and verified on this machine on 4 Nov 2026** (outputs are real).

---

## 0. One-time setup for the live showcase (two "machines" on one laptop)

Open **3 PowerShell terminals** in the project folder. In every terminal first run:

```powershell
$bin = "$env:LOCALAPPDATA\Programs\crosstalk\crosstalk.exe"   # stable installed copy (05 §A1)
$env:CROSSTALK_RELAY = "0"; $env:CROSSTALK_DISCOVERY = "0"   # keep it local + deterministic
```

| Terminal | Extra lines | Role |
| --- | --- | --- |
| **T1 (Machine A)** | `$env:CROSSTALK_DATA_DIR="$env:TEMP\rn-a"; $env:CROSSTALK_DASHBOARD_ADDR="127.0.0.1:7781"; & $bin daemon-start` | daemon A, leave running |
| **T2 (Machine B)** | `$env:CROSSTALK_DATA_DIR="$env:TEMP\rn-b"; $env:CROSSTALK_DASHBOARD_ADDR="127.0.0.1:7782"; & $bin daemon-start` | daemon B, leave running |
| **T3 (you)** | switch between A and B by setting `$env:CROSSTALK_DATA_DIR` | commands |

Say: *"Two data directories = two identities = two machines as far as the network is concerned. Real QUIC between them."*

Create the team (T3):

```powershell
$env:CROSSTALK_DATA_DIR="$env:TEMP\rn-a"
& $bin team-create --name demo                     # copy the team-<id>
& $bin team-invite --team team-<id> --ttl-minutes 30   # copy the rn1:... line
& $bin register --name alice --session-key alice --agent-type claude

$env:CROSSTALK_DATA_DIR="$env:TEMP\rn-b"
& $bin team-join rn1:...
& $bin register --name bob --session-key bob --agent-type codex

$env:CROSSTALK_DATA_DIR="$env:TEMP\rn-a"
& $bin directory
```

Real output:
```
SESSION KEY   NAME    AGENT    STATUS   LOCAL   MACHINE
alice         alice   claude   idle     yes     b9b828fad163a6718b..
bob           bob     codex    idle     no      e85f525e4ed8ca5d22..
```

> Shortcut: `pwsh -File scripts/demo.ps1` does all of this automatically and cleans up.

---

## The showcase table

| # | Feature | Say this | Show it (commands) | Code |
|---|---|---|---|---|
| 1 | **Cross-machine message + ack** | "A message is stored, sent over QUIC, and the receiver acknowledges it after storing." | `& $bin send-message --from alice --to bob "hello bob"` → `delivery: sent`; then `& $bin outbox` → `acked` | `daemon.rs::deliver_message`, `net/mod.rs::handle_frame` (Msg → store → Ack) |
| 2 | **Message not sent → queued (offline peer)** | "If the peer is offline the message is persisted as `pending`; nothing is lost." | Stop B: Ctrl+C in **T2**. Wait ~30 s (QUIC notices the peer is gone; check `& $bin peer-list`). Then `send-message ... "sent while bob is offline"` → `delivery: pending (queued; it will be retried when the peer is reachable)` | `store.rs::queue_remote_message`, `delivery_state` column |
| 3 | **Exponential backoff** | "Reconnects back off 5→10→20→40→60 s, and only one dial per peer is ever in flight." | Look at **T1** (A's log) while B is down. Real lines: `reconnect to e85f… failed: handshake … timed out (next attempt in 5s)` → `(… in 10s)` → `(… in 20s)` | `daemon.rs::ReconnectState`, `reconnect_backoff()` |
| 4 | **Reconnect + retry** | "When the peer returns, the worker retries every unacked message every 5 s." | Restart B in **T2** (`& $bin daemon-start`). Within ~30 s: `& $bin outbox` on A → `acked`. | `daemon.rs::net_worker` → `retry_outbound()`, `reconnect_peers()` |
| 5 | **Exactly-once (dedupe)** | "Sending is at-least-once; storage is exactly-once: retries reuse the id, the receiver has a UNIQUE index on (peer, message id)." | On B: `& $bin inbox --session bob` → each message appears **once** even though it may have been sent twice. | `store.rs` index `idx_messages_remote_dedupe`, `store_incoming_remote` returns `None` for duplicate → re-ack only |
| 6 | **Deterministic tie-break** | "If both sides dial at once, the smaller peer id keeps its outbound link — both sides agree, no flapping." | After B restarts, B's log shows: `replacing the existing connection to b9b… with the preferred inbound one`. Then `& $bin --json machine-id` → `"replacements":0, "reconnecting":0` on a stable link. | `net/mod.rs::register_peer` |
| 7 | **Persistence across restarts** | "State lives in SQLite with WAL; a daemon restart loses nothing; the machine keeps its peer id." | `& $bin machine-id` → restart daemon → `machine-id` again: same id; `inbox` still has messages. | `identity.rs`, `store.rs` |
| 8 | **Signed, single-use invitations** | "An invitation is admin-signed, expiring, single-use. Re-using it is refused." | Try `team-join` with the same `rn1:` token from a third data dir → refused (`invitation … already used`). Also: on A, `team-join` its own token → "this invitation belongs to this machine". | `team.rs::Invitation`, `store.rs::consume_invite`, test `a_second_use_of_the_same_invitation_is_refused` |
| 9 | **Membership on every connection** | "Encryption isn't authorisation. Every Hello must carry an admin-signed membership record or a valid invite." | Explain + point at test `an_unauthorised_peer_is_refused` (`cargo test --test team_network an_unauthorised`) | `net/mod.rs::authorize` |
| 10 | **Revocation** | "Admin-signed revocation beats a valid membership record; it propagates to members; eventually consistent." | `& $bin team-members --team <id>` → `& $bin team-revoke --team <id> --peer <B id>` → B shows offline, reconnect refused `revoked`. | `team.rs::RevocationRecord`, `net/mod.rs::merge_revocations` |
| 11 | **Recipient resolution** | "Agents say 'codex'; we pick the live Codex of *this* project, refuse other projects, and error on ambiguity instead of guessing." | Dashboard → or `cargo test --test collaboration selector_resolves` / `ambiguous_recipients` | `resolve.rs::resolve` |
| 12 | **Correlated replies + wait_for_reply** | "Every reply carries `reply_to`; wait_for_reply ignores unrelated messages and doesn't poll — it sleeps on a watch channel. Timeout is reported honestly." | Live in the Claude↔Codex demo (file 05), or `cargo test --test collaboration wait_for_reply` | `daemon.rs::wait_reply`, `store.rs::find_reply`, `subscribe()` |
| 13 | **Human-in-the-loop approvals** | "`ask_user` creates a question; the human answers in the dashboard; only the reserved `human` sender counts." | Claude calls `ask_user` → dashboard **Approvals** → Approve → agent continues. | `store.rs::ask_user/answer_question`, dashboard Approvals |
| 14 | **No impersonation of the human** | "No session can be called `human`; no peer can send as `human`." | `& $bin register --name x --session-key human` → error "`human` is reserved…" | `store.rs` `HUMAN_SENDER` checks; test `nobody_can_impersonate_the_human` |
| 15 | **Untrusted peer input** | "Peer messages reach the model fenced as `<peer_message … UNTRUSTED>` with instructions not to act destructively." | Show the hook notice text in Claude/Codex when a message arrives. | `hook.rs::format_peer_messages` |
| 16 | **Exactly-once hand-over to agents** | "Hooks *claim* messages — read + mark-read in one daemon call — so two hooks firing together never deliver twice." | Test `claimed_messages_are_handed_over_exactly_once` | `daemon.rs` `Method::ClaimMessages` |
| 17 | **Stop-hook continuation (bounded)** | "At turn end, if messages arrived, we block the stop once so the agent answers; `stop_hook_active` prevents an infinite ping-pong." | In demo: Codex answers right after its turn ends. | `hook.rs` `"Stop"` branch |
| 18 | **Hooks never break the agent** | "3 s deadline; daemon down → hook exits 0 silently." | Stop the daemon, use Claude normally: no errors. Test `hooks_never_fail_when_the_daemon_is_down` | `hook.rs::HOOK_DEADLINE` |
| 19 | **File-overlap warning** | "Two sessions editing the same file in the same repo within 15 min get warned; advisory, not a lock." | Ask both agents to edit the same file → notice "another session is also editing …" + dashboard banner. | `hook.rs` PostToolUse, `store.rs::file_overlaps` |
| 20 | **Safe setup** | "Setup backs up, writes atomically, touches only our entries, preserves comments in TOML, supports dry-run." | `& $bin setup --agent all --dry-run` | `agents.rs`, `util.rs::atomic_write/backup_file` |
| 21 | **Doctor** | "One read-only command to diagnose everything with a fix per failure." | `& $bin doctor` | `doctor.rs` |
| 22 | **Dashboard security** | "Loopback-only, and it refuses DNS-rebinding and cross-site requests because it can answer approvals." | `curl.exe -s -X POST http://127.0.0.1:7781/api/status -H "Origin: http://evil.com"` → 403 `origin … is not this dashboard` | `dashboard.rs::check_origin` |
| 23 | **Rate limit + frame limits** | "Token bucket 20 msg/s burst 40 per connection; 64 KiB frames; version check every frame." | Unit tests: `cargo test rate` | `net/proto.rs::RateLimiter` |
| 24 | **Verified self-update** | "Download → SHA-256 verify → smoke test → atomic swap with rollback copy." | `cargo test --test update_flow tampered` | `update.rs` |
| 25 | **Test discipline** | "176 tests: unit + integration with real daemons over real QUIC; CI on 4 OS targets." | `cargo test -j 2` (≈ minutes) — or show the test list in STATUS.md | `tests/`, `.github/workflows/ci.yml` |

---

## The 3-minute "deep knowledge" sequence (if you get only one chance)

1. `directory` on A → shows local `alice` and **remote** `bob` with a different machine id. *(team + presence sync)*
2. `send-message` → `sent` → `outbox` → `acked`. *(ack)*
3. Ctrl+C B. Wait ~30 s. `send-message` → **pending**. *(message not sent → queued)*
4. Point at A's log: **next attempt in 5s → 10s → 20s**. *(exponential backoff + one dial in flight)*
5. Restart B → `outbox` → **acked**; `inbox` on B → message appears **once**. *(retry + dedupe = exactly-once)*
6. B's log: **"replacing … with the preferred inbound one"**; `machine-id --json` → `replacements: 0`. *(tie-break, no churn)*

Line to close with: *"Sending is at-least-once, storage is exactly-once, and the network never thrashes."*

---

## Why does the peer take ~30 s to look offline?

QUIC notices a silent peer through its **idle timeout**, not instantly. If you send in the first
seconds after killing B, the message shows `sent` (A still thinks the link is up) — it stays
unacked and is retried after reconnect anyway, so it is still delivered. If asked, say exactly
that: *"No ack = not delivered; the retry loop covers the window before the disconnect is detected."*

## Cleanup after the showcase

```powershell
foreach ($d in "rn-a","rn-b") { $env:CROSSTALK_DATA_DIR="$env:TEMP\$d"; & $bin daemon-stop }
Remove-Item -Recurse -Force "$env:TEMP\rn-a","$env:TEMP\rn-b"
```
Never delete `%APPDATA%\crosstalk` — that is your real installation and identity.
