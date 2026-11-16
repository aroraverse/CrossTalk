# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

A developer running two or more AI coding agents on the same project at once (Claude Code, Codex,
Cursor, Kiro CLI, OpenCode, or any MCP client). They open the dashboard mid-task to see which agents
are alive, follow what the agents are saying to each other, and answer the questions agents raise
for a human. Confirmed by the user.

## Product Purpose

Crosstalk is a local-first, peer-to-peer messaging layer for AI coding agents. It replaces the human
acting as a copy-paste router between agents: agents discover each other, message an exact session,
wait for the correlated reply, and stop to ask the human before consequential changes. Success means
the developer never pastes context from one agent into another, and every exchange stays traceable.

## Positioning

No more copy-paste between agents. Agents from different vendors talk directly, each reply is tied
to the request it answers, everything stays on the developer's machines (no cloud service), and only
the human can approve. Confirmed by the user as the claim the product must never lose.

## Operating Context

- One daemon per machine owns all state (SQLite) and serves the dashboard on `127.0.0.1:7777`.
- The dashboard is open beside terminals running agents; it polls the daemon every few seconds.
- Machines join a team with a signed, single-use invitation and connect over encrypted QUIC (iroh).
- The dashboard will be screen-recorded for a short product video.

## Capabilities and Constraints

- Pages: Overview, Sessions, Messages, Approvals, History, Teams, Setup. Every existing page, action
  and `/api/*` call is product function and must keep working (integration tests cover them).
- Technical: one HTML file embedded in the binary (`include_str!`), served offline by the daemon; no
  build step, no web framework, no runtime dependency on a CDN.
- Terms: machine, peer, team, session (one agent conversation), session key, approval, anchor.
- Agents integrate through MCP tools; Claude Code and Codex also through lifecycle hooks.

## Brand Commitments

- Name: Crosstalk.
- Must work in dark mode (binding: it will be screen-recorded); light may follow the system.

## Evidence on Hand

- 183 automated tests; a verified run of four real agents (Claude Code, Codex, Kiro CLI, OpenCode)
  exchanging correlated replies (`demo/e2e-team`). No users, customers, testimonials or benchmarks
  exist; do not invent any.

## Product Principles

1. The agents do the talking; the human makes the call.
2. Every message is traceable to the request it answers.
3. Local and honest: show real state, including what is offline, pending or failed.
4. Any agent can join: nothing is specific to one vendor.
