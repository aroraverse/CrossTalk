# 04 — Same Machine, Two Machines, Two Networks

The one rule that explains everything:

> **One data directory = one daemon = one machine identity (peer id).**
> Sessions (Claude, Codex) live *inside* a daemon. Peers are *other daemons*.

---

## Scenario 1 — One computer, one daemon (the normal case)

```
 Laptop
 ┌──────────────────────────────────────────┐
 │ Claude Code ──MCP/hook──┐                │
 │                         ├─ named pipe ─► daemon ─► SQLite
 │ Codex       ──MCP/hook──┘                │
 └──────────────────────────────────────────┘
```

- Claude and Codex both talk to the **same daemon** over the **named pipe** (IPC). No network.
- Dashboard: **2 sessions, 0 peers.** Correct — a peer is another *machine*, not another agent.
- Delivery is instant (same database).
- **Use it for:** the Claude ↔ Codex collaboration demo.

## Scenario 2 — One computer, two daemons (in two terminals)

```
 Laptop
 ┌───────────────────────────────────────────────────────────────┐
 │ Terminal 1: DATA_DIR=rn-a → daemon A (key A, pipe A, :7781)    │
 │                   ▲  real QUIC over 127.0.0.1 / LAN IP          │
 │                   ▼                                              │
 │ Terminal 2: DATA_DIR=rn-b → daemon B (key B, pipe B, :7782)    │
 └───────────────────────────────────────────────────────────────┘
```

**How:** give each terminal a different `CROSSTALK_DATA_DIR` (and a different
`CROSSTALK_DASHBOARD_ADDR` so both dashboards can run). Each daemon:
- generates its **own `machine.key`** → different peer id,
- derives its **own pipe name** from the data-dir hash → they never collide,
- binds its **own UDP port** for iroh.

Then `team-create` on A, `team-invite`, `team-join` on B. They connect with **real encrypted QUIC** —
the exact code path two physical machines use.

**Why this is beneficial:**
1. **Test the network without a second computer** — invite, join, ack, offline queue, retry,
   backoff, revocation, tie-break all exercised for real. The automated `team_network` tests do
   exactly this (with 2–3 daemons).
2. **Deterministic demos** — set `CROSSTALK_RELAY=0` and `CROSSTALK_DISCOVERY=0` so
   nothing depends on Wi-Fi or the internet. Perfect for an interview.
3. **Kill one, keep the other** — Ctrl+C one terminal to simulate "teammate's laptop went to
   sleep" and show the message queue + reconnect.
4. **Isolation** — temp data dirs never touch your real `%APPDATA%\crosstalk`.

**What NOT to do:** point both daemons at the *same* data dir. The second one fails fast
(`first_pipe_instance` on the named pipe) — that's deliberate: two owners of one SQLite file would
corrupt the single-source-of-truth rule.

Shortcut: `pwsh -File scripts/demo.ps1` runs Scenario 2 end-to-end.

## Scenario 3 — Two computers on the same network (same Wi-Fi / office LAN)

```
 Laptop A (Wi-Fi 192.168.1.13) ◄──── direct UDP/QUIC on the LAN ────► Laptop B (192.168.1.20)
```

- Each laptop runs its own daemon (default data dir is fine — they're different machines).
- A: `team-create`, `team-invite` → send the `rn1:` token privately (chat/email).
- B: `team-join rn1:…`. **No IP typed by hand** — the token carries A's addresses
  (`EndpointAddr`: peer id + local IPs + relay URL).
- What's used: **iroh direct path** on the LAN; **mDNS** (`net/discovery.rs`) helps find each
  other's current address if it changed (e.g. DHCP gave a new IP).
- Windows prompts once to allow `crosstalk` through the firewall → allow on **Private**
  networks. If declined, it still works through the relay.

## Scenario 4 — Two computers on different networks (home vs office, behind routers)

**Yes, it works.** This is what the relay is for.

```
 Laptop A (home, behind NAT) ──┐                       ┌── Laptop B (office, behind NAT)
                               └──► iroh relay server ◄─┘
                                   (forwards encrypted bytes)
           …then iroh tries hole punching → direct path if possible
```

Step by step:
1. On start, each endpoint picks a **home relay** (we observed `aps1-1.relay.n0.iroh.link`) and
   learns its public IP. These go into its `EndpointAddr`.
2. The invitation carries A's relay URL → B can reach A even though A has no public IP.
3. Connection starts **through the relay** (works through almost any NAT/firewall since it's
   outbound traffic).
4. iroh simultaneously tries **UDP hole punching**; if it succeeds, traffic moves to a **direct**
   path automatically (faster). If not (strict corporate NAT), it stays on the relay.
5. Either way it's **end-to-end encrypted** with the machines' keys — the relay sees only
   ciphertext and cannot read or forge messages.
6. After the first handshake, peers exchange **current addresses in Hello/Welcome** and remember
   them, so restarts and IP changes heal automatically.

Honest limits (say them before they ask):
- Uses n0's public relays by default; a company would run its **own iroh relay** for control.
- Not every NAT/relay-only/cross-host-mDNS scenario has been tested on physical networks —
  automated tests use local daemons. (See STATUS.md.)

## Comparison table (memorise this)

| | Same machine, 1 daemon | Same machine, 2 daemons | 2 machines, same LAN | 2 machines, different networks |
|---|---|---|---|---|
| Identities | 1 | 2 | 2 | 2 |
| Agents talk via | named pipe | pipe → own daemon → QUIC | pipe → daemon → QUIC | pipe → daemon → QUIC |
| Path between daemons | none | loopback / LAN IP | direct LAN | relay → maybe hole-punched direct |
| Discovery | n/a | invitation | invitation + mDNS | invitation + relay |
| Peers in dashboard | 0 | 1 | 1 | 1 |
| Typical use | daily Claude↔Codex | testing & demos | team in an office | remote team |

## Likely follow-ups

- **"Why not just a central server / WebSocket?"** — Then all code-review conversations flow
  through a third party, someone must host it, and it's a single point of failure. P2P keeps data
  on the team's machines; iroh gives us NAT traversal and encryption without running servers. The
  relay is only a fallback pipe and can't read content.
- **"What if the admin is offline?"** — Existing members still connect to each other with their
  signed records; only *new joins* (redeeming an invite) need the admin.
- **"How does B know A's new IP after a restart?"** — Hello/Welcome frames carry the current
  address; it's saved in the `peers` table; through a relay the peer id alone is enough.
- **"Can a random person on the Wi-Fi join?"** — No. mDNS only resolves addresses. Joining
  requires an admin-signed invite; every connection needs an admin-signed membership record.
