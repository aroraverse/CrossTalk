# 06 — Demo Talk Track (what to say, step by step)

Short paragraphs to say **after each big step**. 2–3 sentences each, so speak, don't read.
Commands and screens are in [05_LIVE_DEMO_CLAUDE_CODEX.md](05_LIVE_DEMO_CLAUDE_CODEX.md).

---

## Opening (before anything runs)

> "I'll show this in two parts. First two dummy agents, Alice and Bob, so you can see the engine: how a
> message moves, gets acknowledged, and survives a machine going offline. Then the same engine doing
> real work, with Claude Code and Codex collaborating on a codebase while I approve the risky decision."

---

## PART 1 — Alice & Bob (the engine)

**After step 1: two machines start**
> "Each window is a separate machine: its own daemon, its own SQLite database, its own ed25519
> identity. The daemon is the single owner of state; agents never touch the database directly."

**After step 2: team + agents**
> "Machine A created a team and signed a single-use invitation; B joined with it, and no IP address was
> typed. Every connection must show an admin-signed membership, because encryption alone isn't
> permission. Now A's directory shows Bob as a remote agent on another machine."

**After step 3: message + ack**
> "Alice's message was stored first, then sent over encrypted QUIC. It turned 'acked' only when Bob's
> machine confirmed it had *stored* it. Sent and delivered are different states, and I track both."

**During the ~30-second wait in step 4: Bob goes offline**
> "Bob's machine just died. QUIC notices a silent peer through an idle timeout, which takes about thirty
> seconds. Meanwhile, nothing on Alice's side is lost."

**After step 4: pending + backoff**
> "The message is now 'pending', queued on disk rather than dropped. In Alice's log, reconnect attempts
> back off five, ten, twenty seconds, capped at sixty, with only one dial in flight. That way a dead
> peer costs almost nothing."

**After step 5: Bob returns**
> "Bob came back, the link reconnected, the queued message was retried and acknowledged. Retries reuse
> the same message id, and the receiver de-duplicates on it, so Bob has each message exactly once. So:
> at-least-once sending, exactly-once storage."

**Transition**
> "That's the engine. Now let me show it doing real work."

---

## PART 2 — Claude & Codex (the real thing)

**After start: both agents visible on the dashboard**
> "These are two real agents from two different companies, in the same project folder. Each one
> registered itself through a lifecycle hook and talks to the daemon through MCP tools. Two sessions,
> zero peers: same machine, one daemon."

### Demo 1 — Human approval

**After Claude calls `ask_user` and blocks**
> "Claude hit a breaking decision, so instead of guessing it opened a checkpoint and is now blocked,
> waiting for me. That question is this card on the Approvals page."

**After you click Send response**
> "My answer is stored as a message from the reserved `human` sender, which no agent or peer can
> impersonate. Claude woke up the moment it landed, because `wait_for_reply` sleeps on an event and
> doesn't poll."

### Demo 2 — Full loop

**After Claude resolves and messages Codex**
> "Claude asked for 'codex' and got the live Codex of *this* project, not one from another folder. If
> two matched, it would refuse to guess. The message got an id, and Claude is now waiting for a reply
> to *that* id specifically."

**After Codex's review arrives**
> "Codex received it through its hook, marked as untrusted input, reviewed the real code and replied
> with `reply_to` set to the request's id. Those are real bugs: float money, expired coupons still
> applying, and a negative total at 150% off."

**After the approval and implementation**
> "The breaking change went through me first. Claude then implemented only what I approved and wrote the
> tests Codex asked for."

**When you type the nudge into Codex for the verification request**
> "Codex was idle, and a hook can't wake an idle model. That's a platform limit I designed around rather
> than hid. One prompt, and its hook hands over Claude's request. In production, the opt-in runner or an
> orchestrator like Alan triggers that turn."

**After Codex verifies**
> "A different model from a different vendor verified the result by actually running the tests. And
> every step is in Messages and History, searchable and stored locally."

---

## END — what happened, end to end (say this with the dashboard open)

> "Every agent talks to one small daemon on its machine through a thin MCP adapter and lifecycle hooks.
> The daemon stores every message in SQLite, picks the right recipient, and correlates every reply to
> its question. When agents are on different machines, daemons talk peer to peer over encrypted QUIC,
> with acknowledgements, retries, backoff and de-duplication. That's what Alice and Bob showed. On top
> of that, Claude and Codex can find each other, ask for a review, wait for the *right* answer, and
> stop for a human decision that only I can give. The dashboard is a window into the same daemon:
> Overview for who's alive, Messages for the threads, Approvals for my decisions, History for search.
> No message leaves this laptop unless a teammate's machine is in the team."

**If you have 10 seconds left, one line:**
> "Agents were islands. This gives them a reliable, secure way to talk, with the human still in charge."
