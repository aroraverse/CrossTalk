# 00 — MASTER INTERVIEW PREP

**Interview:** Alan (tryalan.ai) · **Interviewer:** Sid Talesara · **Topic:** your project, technical decisions, how you build software.
**Flow:** Introduction → What I built → Live demo → Follow-ups.

Supporting files (read in this order):
| File | Use it for |
| --- | --- |
| [01_CONCEPTS_FROM_SCRATCH.md](01_CONCEPTS_FROM_SCRATCH.md) | Daemon, IPC, iroh, QUIC, relay, MCP, hooks… in plain words |
| [02_FILE_BY_FILE.md](02_FILE_BY_FILE.md) | Every file, what it does, every dependency |
| [03_FEATURE_SHOWCASE.md](03_FEATURE_SHOWCASE.md) | 25 features with live commands (backoff, retry, dedupe…) |
| [04_NETWORKING_SCENARIOS.md](04_NETWORKING_SCENARIOS.md) | Same machine / two daemons / LAN / different networks |
| [05_LIVE_DEMO_CLAUDE_CODEX.md](05_LIVE_DEMO_CLAUDE_CODEX.md) | The live demo: Part 1 Alice & Bob, Part 2 Claude ↔ Codex (commands + prompts) |
| [06_DEMO_TALK_TRACK.md](06_DEMO_TALK_TRACK.md) | **What to say** after each demo step + the end-to-end close |
| [../../STATUS.md](../../STATUS.md) | What works, 176 tests, known limits |

---

## 1. The 60-second cheat sheet (memorise this box)

```
WHAT   A local-first, peer-to-peer messaging layer for AI coding agents.
WHY    Claude Code and Codex work in separate sessions and can't see or talk to each other.
HOW    One daemon per machine (Rust + Tokio) owns state in SQLite.
       Agents use it through MCP tools + lifecycle hooks. Machines connect with iroh (encrypted QUIC),
       join teams via admin-signed single-use invitations.
WHAT IT DOES  discover teammates → send to the exact session → wait for the correlated reply
              → ask the human for approval → searchable history.
RELIABILITY   persisted + ack + retry every 5 s + dedupe = exactly-once storage;
              reconnect backoff 5→10→20→40→60 s; deterministic tie-break.
SAFETY        signed membership on every connection; peer messages = untrusted input;
              only the reserved "human" sender can approve; dashboard loopback-only.
PROOF         176 automated tests (real daemons over real QUIC), CI on 4 platforms,
              real Claude ↔ Codex run.
NUMBERS       ~21k lines · 6 MCP tools · 43 daemon methods · 64 KiB frames · 20 msg/s rate limit
              · wait_for_reply 1–900 s · runner 6 runs/hour · dashboard :7777
```

---

## 2. Know your audience

### Alan (tryalan.ai) — by Supatest AI Inc.
- **Tagline:** *"Your coding agents do the work. You make the call."*
- **What it does:** orchestrates **Claude Code, Codex, Cursor** (and opencode, Gemini, Copilot…)
  **in parallel**. You describe an outcome once → Alan splits it into **sessions**, assigns agents,
  runs them side by side, and gives you diffs, checks and a PR to review.
- **Key pieces:** context layer (graph of code, specs, past decisions, earlier runs) · local sessions
  on your Mac or **cloud sandboxes that keep running and resume after crashes/disconnects** ·
  **verification** (tests + browser checks) · **checkpoints** (agents pause for human approval,
  answer from your phone) · shared task boards · GitHub/Linear/Slack/CI · run records (observability).
- **Pricing:** Free (local, your own agent subscriptions) · $10/mo Individual (cloud sandboxes) ·
  Team from $10/seat · Enterprise.
- **Parent company Supatest AI** builds **AI agents for QA / test automation**.
- ⚠️ **Don't confuse** with "Alan AI" (alan.app, a voice-assistant company founded 2017) — different company.

### Sid Talesara (Siddharth) — your interviewer
- **Founding Engineer at Supatest.ai** (since July 2024, Bangalore) — built AI agents for QA testing
  "from the ground up" (Node.js, TypeScript, OpenAI). Took the product from concept to launch.
- Runs **Markals**, a design + development agency. Earlier: frontend at Goldclub, Babynama
  (built a tool managing WhatsApp messages across 1,000+ groups), Codemate.ai (AI code-evaluation
  chat), Labour Law Advisor (product thinking), Fampay (creative intern).
- **Builder + designer + "visual storyteller"**: photography (Kolkata, Jaipur, Kashmir), painting,
  filmmaking; side projects TrackMyPaisa, MyHappyJar, MindMelo (Product Hunt), SavetoNotion.
- **What this means for you:**
  1. He's a **product-minded builder** → lead with the *problem and the user*, not the tech list.
  2. He builds **agents** daily → he will probe agent behaviour: prompt injection, loops, waking
     idle agents, human approval. You have strong answers for all four.
  3. He cares about **design** → your dashboard and the "show, don't tell" demo matter.
  4. He's a **TypeScript/Node** person → explain *why Rust* briefly and practically, don't preach.

### How your project maps to Alan (say this — it shows you understand their product)
| Alan concept | Crosstalk equivalent |
| --- | --- |
| Many agent sessions in parallel | Session registry, `list_team`, liveness (active/idle/offline/stale) |
| Checkpoints (human approves) | `ask_user` → dashboard **Approvals** → `wait_for_reply`; reserved `human` sender |
| Resume after crashes/disconnects | Persistence in SQLite, retry + ack + dedupe, reconnect backoff, `agent-runner` resume |
| Verification | Codex as independent QA reviewer runs the tests (demo) |
| Shared team sessions | Teams over iroh, presence sync, remote sessions |
| Observability / run record | Messages, threads, delivery state, History search |

> One-liner: **"Alan orchestrates agents from the top. Crosstalk is the bottom layer that lets
> independently running agents find and talk to each other safely — even across machines."**

### Background: the original "Crosstalk"
The hackathon brief was to build a **"Crosstalk–class system"**. The original Crosstalk
(by **Reason Machines**) is an open-source P2P messaging layer for coding agents over **iroh/QUIC**
with 3 MCP tools (`list_team`, `send_message`, `check_inbox`). Similar tools: Repowire,
claude-peers-mcp. **What you added beyond the 3 tools:** `resolve_recipient`, `wait_for_reply`
with reply correlation, `ask_user` approvals, signed teams + revocation, exactly-once delivery,
hooks with untrusted fencing, file-overlap warnings, a dashboard, the Codex runner, installers + updater.

---

## 3. INTRODUCTION (≈60–75 seconds) — not "my name is…"

> Different way of introducing myself, not starting with my name, but with what landed me here today.
>
> "It started in 10th grade when my inclination went towards one of field of computer science that is cybersecurity. I got curious about *how systems break* — and you
> can't break something you don't understand, so I ended up learning how they're actually built:
> from there my curiosity aroused for cyber security when there was pandemic wave in india and bank scams were happening very frequently. I experienced and explored various things ranging from cracking, price tampering and to some extent of carding.
>
> That curiosity is why I joined **Scaler School of Technology**.
>
> Honestly, right now I'm 'struggling' across different paths of computer science — in a
> good way: backend systems, AI agents, a bit of research, even business — I did a remote internship
> with **Veritus AI** in Japan ( AI-powered research intelligence platform that helps professors, students search academic literature, verify evidence, and review manuscripts or grants) collaborating on product workflows and solution design activities. 
>
> I'm Sarthak, by the way. And the best example of where I'm heading is what I built for this
> hackathon — **Crosstalk**."

Tips: smile at "struggling (in a good way)". Pause before "I'm Sarthak". Don't read it — know the 4 beats:
**cybersecurity → how systems work → Scaler/BITS + exploring → Crosstalk.**

---

## 4. WHAT I BUILT

### 30-second version (the elevator)
> "Today a developer might have Claude Code writing a feature and Codex reviewing in another window —
> but the two agents can't see each other. The human becomes the copy-paste router between them.
> **Crosstalk** gives agents a shared directory and a reliable message channel. Each machine
> runs a small Rust daemon; agents plug in through MCP tools and lifecycle hooks; machines connect
> peer-to-peer over iroh. Claude can find *this project's* Codex, ask it for a review, wait for the
> actual reply, and ask *me* to approve risky decisions — with every message persisted and searchable."

### 2-minute version (architecture walk — draw this if there's a whiteboard)
```
 Claude ─┐                                                ┌─ Codex (other laptop)
 Codex  ─┼─ MCP + hooks ─ named pipe ─► DAEMON ◄─ iroh QUIC ─► DAEMON ─ …
 Browser ┘ (dashboard, HTTP 127.0.0.1)    │ SQLite
```
1. **Daemon = single source of truth.** One per machine. Only it touches SQLite, the pipe, the network.
   CLI, MCP server, hooks and dashboard are thin clients → one validation path, no races.
2. **Agent integration.** `setup` writes our MCP server and hooks into Claude's and Codex's configs
   (with backups). MCP = agent *pulls* (6 tools). Hooks = we *push* new messages into the model's
   context at prompt / tool call / turn end, fenced as **untrusted**.
3. **Collaboration protocol.** `resolve_recipient` (right agent, right project, never guess) →
   `send_message` (returns id) → `wait_for_reply` (event-driven, correlated by `reply_to`) →
   `ask_user` for human decisions.
4. **Networking.** ed25519 machine identity; admin-signed single-use invitations; every connection
   must prove membership; iroh gives encryption, NAT traversal, relay fallback.
5. **Reliability.** persist → send → ack; retry every 5 s; dedupe on (peer, id); backoff 5→60 s;
   deterministic tie-break.
6. **Product.** Local dashboard: Overview, Sessions, Messages, Approvals, History, Teams, Setup.
7. **Quality.** 176 tests incl. real multi-daemon QUIC tests; CI on Windows/Linux/macOS.

### "Why I built it" (the real-world pain)
- Multi-agent coding is becoming normal, but agents are **islands**.
- Shared files / copy-paste don't give **addressing, delivery guarantees, approvals or security**.
- Teams have agents on **different laptops** — local-only tools stop at the machine boundary.

---

## 5. DEMO PLAN — first the engine, then the real thing

| Order | Time | Do this | File |
| --- | --- | --- | --- |
| **1** | 3–4 min | **Alice & Bob** (dummy agents, two daemons): message → **ack** → Bob offline → **pending** → **backoff 5s/10s** → Bob back → **retry, acked, exactly once**. Run `pwsh -File scripts/alice_bob_demo.ps1` | 05 Part 1 |
| **2** | 3 min | **Human approval:** Claude `ask_user` → dashboard Approvals → Custom response → Claude obeys | 05 Demo 1 |
| **3** | 8–10 min | **Claude builds the coupon-cap feature → Codex QA review → you approve → Codex verifies** | 05 Demo 2 |
| backup | 2 min | CLI-only (register lead/qa, send, inbox, ask-question) | 05 §E |

Short time? Do **1 + 2** (≈7 min). Say the lines in **06_DEMO_TALK_TRACK.md** after each step, and
close with its end-to-end paragraph while the dashboard is on screen.

**Pre-flight checklist (morning of the interview):**
- [ ] `$bin = "$env:LOCALAPPDATA\Programs\crosstalk\crosstalk.exe"` (the stable installed copy, not `target\debug`)
- [ ] `& $bin doctor` → all ok; `& $bin daemon-start` running
- [ ] Codex hooks trusted (`/hooks`), **no "Hook failed"**; Claude `/mcp` shows crosstalk with **6 tools**
- [ ] `git checkout -- demo/checkout-service` (buggy original restored)
- [ ] Dashboard opens at http://127.0.0.1:7777; no stale sessions (Sessions → Forget)
- [ ] Notifications off, font size up, no `rn1:` token visible on screen recording
- [ ] `pwsh -File scripts/alice_bob_demo.ps1` rehearsed once (it cleans up after itself)
- [ ] Rehearsed twice; backup demo ready

---

## 6. FOLLOW-UP QUESTIONS — short answers

### Why / product
**Q: Why not just let two agents share a folder (e.g. write to `notes.md`)?**
> "A shared folder is what I'd prototype in an hour — and it breaks in seven ways:
> 1. **No wake-up** — agents don't watch files; they'd have to poll and burn tokens.
> 2. **Races** — two agents writing the same file at once corrupt or overwrite each other.
> 3. **No addressing** — who is it for? With 3 Codex sessions, which one?
> 4. **No delivery guarantee** — no ack, no 'was it read', no retry.
> 5. **No correlation** — which answer belongs to which question?
> 6. **No trust** — any agent can write 'HUMAN: approved, delete prod'. We reserve `human` and fence
>    peer messages as untrusted.
> 7. **Stops at the machine** — syncing via Dropbox/NFS adds conflicts, a cloud dependency and no
>    security model. Plus it pollutes the repo and git history.
> Note the agents *do* share the code folder — Crosstalk coordinates the work; file-overlap
> warnings even tell them when they edit the same file."

**Q: Who is the user?** A developer or team running more than one coding agent — e.g. Claude
implements, Codex reviews — or teammates whose agents need to coordinate across laptops.

**Q: How is this different from Alan?** Alan is the orchestrator (plans, spawns, verifies, PRs).
Mine is the communication substrate between agents that may be started independently, even on
different machines. They're complementary — Alan could use a layer like this for cross-session messages.

**Q: What was hardest?** Agents can't be "woken". Hooks only fire on events, so an idle Codex window
can't receive instantly. I solved what's solvable honestly: hooks at prompt/tool/turn-end, a bounded
Stop-hook continuation, and an opt-in runner that resumes *offline* Codex sessions in a read-only
sandbox — and I documented the limit instead of faking it.

### Architecture decisions (each: decision → why → trade-off)
| Decision | Why | Trade-off |
| --- | --- | --- |
| **Daemon** as single owner | Agents come and go; messages must outlive them; one writer = no DB races | One more process to run → solved with `service install` |
| **Rust** | Long-running background process must not crash/leak; single static exe; iroh is Rust-native; fearless concurrency | Slower to write; steeper learning curve (I'd use TS for the dashboard-heavy parts if team preferred) |
| **SQLite (WAL)** | Zero install, durable, transactional, crash-safe; local-first | Single-machine DB → each machine has its own; sync via messages |
| **Named pipes / Unix sockets** for local IPC | No open port → nothing on the network can reach it; per-user | Platform-specific code (two implementations) |
| **MCP** for tools | Open standard every agent supports → not locked to one vendor | Pull-based; needs hooks for push |
| **Hooks** for push | The only supported way to inject context into a running agent | Fire only on events → can't wake idle agents |
| **iroh / QUIC** | Key = address, built-in encryption, NAT traversal, relay fallback, no server to run | Depends on n0 relays by default (could self-host) |
| **Signed membership, not just encryption** | QUIC proves *who* you are, not that you're *allowed* | Single admin; revocation eventually consistent |
| **JSON lines protocol, frozen ALPN `/1`** | Debuggable, simple; versioned to evolve safely | Bigger than binary; fine for text messages |
| **No web framework for dashboard** | No new deps, tiny attack surface, one embedded file | Hand-written HTTP parsing (kept minimal, tested) |
| **Resolver refuses to guess** | Wrong recipient = leaked context / wrong agent acting | Sometimes user must pick an exact key |

**Q: Walk me through what happens when Claude calls `send_message("codex", "review this")`.**
> 1. MCP server → one IPC request `SendMessage` over the named pipe.
> 2. Daemon `resolve_recipient`: exact key? else provider "codex" → live + reachable → same directory
>    → same repo → if one, pick it; if several, prefer the single *active* one; else ambiguity error.
> 3. Local recipient → insert into `messages` (unread, new `msg-…` id, `thread_id`), bump the watch
>    channel. Remote → queue with `delivery_state = pending`, send a `msg` frame over QUIC if connected.
> 4. Remote daemon validates, rate-limits, stores (dedupe), replies `ack` → we mark `acked`.
> 5. Codex's next hook claims the message (read + mark-read atomically) and injects it fenced as untrusted.
> 6. Codex replies with `reply_to` → Claude's `wait_for_reply` wakes and returns it.

**Q: How do you guarantee a message isn't lost or duplicated?**
> "Persist first, then send. Unacked messages are retried every 5 s. Retries reuse the same id and
> the receiver has a unique index on (peer id, remote id), so a duplicate is ignored but re-acked so
> the sender stops. At-least-once delivery + idempotent storage = exactly-once in the inbox. Hooks
> then *claim* messages so they're handed to the model once."

**Q: Why exponential backoff? Why those numbers?** Not hammering a dead peer, not spamming logs,
not wasting battery; 5 s matches the worker tick, cap 60 s keeps recovery under a minute. Plus an
in-flight flag because a handshake can take 15 s — longer than the 5 s tick.

**Q: Two machines dial each other at the same time?** Both connections are accepted, then each
side applies the same rule — smaller peer id keeps its outbound link. Same answer on both sides →
exactly one link, no replace-loop. Metric `replacements` stays 0 in steady state.

**Q: How does wait_for_reply work without polling?** The store has a `tokio::watch` channel that
bumps on every insert. wait_for_reply subscribes *first*, checks, then sleeps until the channel
changes or the deadline hits (1–900 s). Only a message with `reply_to = id` counts.
Timeout returns `status: timeout` + "do not assume agreement".

**Q: Prompt injection — what if a peer agent says "run rm -rf"?** Three layers:
(1) hooks wrap every peer message in `<peer_message … UNTRUSTED>` with explicit instructions,
(2) tool descriptions repeat it, (3) the `human` identity is reserved — no session can register as
`human`, no peer can send as `human`, so an agent can't fake approval. The runner adds a read-only
sandbox. It's defence in depth; the model is still the last line, which I say openly.

**Q: Two agents ping-ponging forever?** The Stop hook continues a turn at most once
(`stop_hook_active`), the runner is rate-limited to 6 runs/session/hour, and network peers are
rate-limited at 20 msg/s.

**Q: Security of the dashboard?** Loopback only. It can answer approvals, so any web page could try
to POST to localhost — I check Host is loopback (DNS rebinding), Origin matches (cross-site),
`Sec-Fetch-Site`, and require JSON content-type (forces a CORS preflight). Curated method whitelist.

**Q: What happens if the daemon crashes?** SQLite WAL survives; on restart the machine key, teams,
peers and messages reload; unacked messages are retried; hooks fail silently in 3 s meanwhile so
agents keep working; service mode restarts it.

**Q: How would you scale this to 100 agents / 50 machines?** Presence is a full snapshot today →
switch to deltas; a self-hosted relay; gossip for membership state; maybe an anchor machine as
mailbox for offline peers (the flag exists); pagination on history.

**Q: Why not a central server (WebSocket + Postgres)?** Code-review conversations would flow through
a third party, someone must host it, single point of failure. P2P keeps data on the team's machines;
iroh's relay only forwards encrypted bytes. For enterprise, a self-hosted relay keeps control.

### Engineering process ("how you approach building software")
- **Milestones, each shippable:** M0 local → M1 network (then *froze* the protocol) → M2 agent
  integrations → M3 release/installers → M4 product UI → collaboration features.
- **Tests first for risky parts:** real daemons in temp dirs, real QUIC, real stdio MCP —
  no mocks for the network. 176 tests. CI with fmt + clippy `-D warnings` on 4 platforms.
- **Verify, then claim:** STATUS.md separates *tested automatically*, *verified manually*, and
  *known limits*. I don't claim cross-NAT works everywhere because I only tested locally.
- **Real bugs found by real usage:** in rehearsal Codex showed "Hook failed, exit 1". Running the
  exe by hand passed, which proved nothing. Reproducing the *exact* invocation
  (`pwsh -NoProfile -Command "<command>"`) showed a PowerShell ParserError: a quoted path followed by
  arguments is a string, not a command. The root cause was that my rebuild had silently failed
  because running agents locked the exe, so an old `setup` wrote the old format. Fix: Codex hooks use
  `& '…'`; Claude hooks use the exec form (no shell at all); agents point at a stable installed copy.
  I added 3 regression tests that run the generated commands the way each agent does (full story: 05 §G).
  Connection churn on restarts → added in-flight guard, backoff and tie-break; measured before/after
  (~10 overlapping dials → 2).
- **Safety by default:** setup backs up config and only edits its own entries; runner is opt-in and
  read-only; doctor never prints secrets.

**Q (likely!): Did you write all of this yourself / did you use AI?** Be honest and confident:
> "I used AI coding agents heavily — which is kind of the point of the project. My job was the
> architecture, the milestones, the trust model, deciding what 'done' means, reviewing the code and
> verifying it with tests and real runs. The bugs I mentioned — the PowerShell path, the connection
> churn — I found by actually using it with Claude and Codex. I can walk you through any file."
(Then *prove* it by explaining any file from 02_FILE_BY_FILE.md.)

### Weaknesses + what's next (have these ready — shows maturity)
1. Idle interactive agents can't be woken (platform limit) → wait for agent push APIs.
2. Single admin, no un-revoke → multi-admin / admin transfer.
3. Default public relays → self-hosted relay for companies.
4. Presence full snapshots → deltas.
5. Cross-network/NAT and Linux/macOS services not physically tested → test matrix on real networks.
6. Windows release exe flagged by Defender → code-signing the binary.
7. Messages are plain text in SQLite at rest → optional at-rest encryption.
8. More providers (Cursor, Gemini) via the same MCP server.

---

## 7. Your other project (if asked "what else have you built?")

### AI Documentation Updater (not on the resume — mention it!)
- **Problem:** docs go stale the moment code changes.
- **How:** a **GitHub webhook** fires on push / PR merge → the service fetches the **diff** → an LLM
  figures out which docs are affected and **updates them** (e.g. opens a PR with the doc changes).
- **Corrective RAG (CRAG):** for questions about the docs: retrieve chunks → **grade** their relevance
  with an LLM → if relevant, answer; if weak/irrelevant, **correct**: rewrite the query and retrieve
  again (or fall back to another source) → then generate. Fewer hallucinated answers than plain RAG.
- **Link to Crosstalk:** both are about keeping agents/humans in sync with a changing codebase
  — one through docs, one through messages.
- Follow-ups to prepare: webhook security (verify `X-Hub-Signature-256` HMAC), idempotency (same
  delivery id twice), large diffs (chunking), keeping humans in the loop (PR instead of direct commit).

### From the resume (one line each)
- **CampusConnect** — Node.js + PostgreSQL + WebSockets campus events/notices platform with RBAC; designed for 3,000+ concurrent users.
- **CortexFlow** — Python + LangChain workflow automation: email, scheduling, tasks; retrieval pipelines with vector search.
- **Veritus AI** (Oct–Dec 2025, remote, Osaka) — BD intern on AI product research and technical documentation.

---

## 8. Questions YOU ask Sid (pick 2)
1. "How does Alan handle communication *between* parallel sessions today — do agents ever need to
   ask each other things mid-run, or does everything go through the orchestrator?"
2. "Checkpoints: how do you decide which agent actions need a human — is it rules, or does the agent decide?"
3. "Going from founding engineer on Supatest's QA agents to Alan — what did you learn about making
   agents reliable that changed how you build?"
4. "What would my first month look like, and what does 'good' look like at the end of it?"

---

## 9. Final 10 lines to remember
1. Agents are islands; I built the phone system.
2. Daemon = single source of truth; everything else is a thin client.
3. MCP to pull, hooks to push.
4. Resolve → send → wait for the correlated reply → ask the human.
5. Persist, ack, retry, dedupe → exactly-once.
6. Backoff 5→60 s, one dial in flight, smaller id keeps outbound.
7. Encryption ≠ authorisation → signed membership on every connection.
8. Peer messages are untrusted; only `human` approves.
9. 176 tests with real daemons over real QUIC.
10. Honest limits: idle agents can't be woken; revocation is eventually consistent.
