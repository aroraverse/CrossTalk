# Video brief: "A day with four agents" (Crosstalk time-lapse)

This is the complete brief for a video agent (Emergent or any editor). Everything needed to make the
video is in this file plus the attachments listed at the end. Read sections 1 to 3 first.

## 1. What to make

A **66-second macOS screen time-lapse, split screen**: one developer's 10-hour workday (09:00 to 19:00)
run two ways at the same time, with the same four AI coding agents and the same eight tasks.

- **Left half: copy-paste.** The developer is the clipboard. Every plan, diff and test log is copied
  from one agent's terminal and pasted into the next. Context windows fill up, agents forget, stale
  pastes break tests. The list is finished at **18:55**: 120 pastes, 9 context resets, 5 red test runs.
- **Right half: Crosstalk.** The agents talk to each other over Crosstalk. The developer approves two
  decisions from desktop notifications, works on other things, and the list is finished at **15:15**.
  The developer logs off at 17:30 while the left side is still relaying logs.
- A calm male Indian-English voiceover explains what is happening; **subtitles match it word for word.**
- Ambient, minimal music. No face, no webcam, no "simulated" label, no title cards before the end.

Hard requirements:
- Duration about 60-70 s total (the day itself is 57 s = **632x** time-lapse). 1920x1080 master and a
  1080x1080 cut. 30 fps. Burned-in subtitles. Voiceover + music + light sound effects.
- It must look like a real macOS screen recording that was sped up (see section 4 and the reference
  video): real-looking windows that overlap, open and close; a ticking menu-bar clock; a cursor.
- Nothing on screen may show secrets, invitation strings (`rn1:...`), emails or personal paths.

## 2. The product (so the story makes sense)

Crosstalk is a small local daemon plus an MCP server that lets AI coding agents on one machine (or
across machines, peer to peer) message each other directly. Any MCP agent can join: Claude Code,
Codex, Cursor, Kiro CLI, OpenCode are set up by `crosstalk setup`. Agents get six tools:
`list_team`, `resolve_recipient`, `send_message`, `wait_for_reply`, `check_inbox`, `ask_user`.
An agent asks another by name ("codex", "kiro"), and `wait_for_reply` returns the reply that answers
**that exact request**. Anything risky goes to the human through `ask_user`, answered in a local
dashboard (http://127.0.0.1:7777; pages: Overview, Live, Sessions, Messages, Approvals, History,
Teams, Setup) or, in this video's story, from a desktop notification.

**The problem it removes:** without it, when agent A needs agent B's work, the developer copies text
between terminals. Each paste eats the receiving agent's context window, goes stale the moment the
other agent changes something, can leak secrets sitting in logs, and leaves no record of who decided
what. The video's job is to make that cost visible and the alternative obvious.

Tagline: **Your agents talk. You make the calls.** Repo: github.com/aroraverse/reason-network.

## 3. The day (what both sides do)

Same `TODAY.md` on both sides, eight tasks, ticked as they finish:

| # | Task | Left (copy-paste) done | Right (Crosstalk) done |
| --- | --- | --- | --- |
| 1 | Standup & triage | 09:23 | 09:23 |
| 2 | #118 Flaky retry test | 10:11 | 09:40 |
| 3 | #121 Message search (FTS index) | 12:21 | 11:45 (schema change approved by the human at 11:30) |
| 4 | #123 Desktop notifications | 14:32 | 12:50 (agents keep working through lunch) |
| 5 | #124 File handoff (attach a file or diff to a message) | 15:58 | 14:00 |
| 6 | #126 Threads view in the dashboard | 17:18 | 14:45 |
| 7 | Review teammate's PR #130 | 18:17 | 14:57 |
| 8 | Release 0.2 (changelog, tag) | **18:50** | **15:15** (tag approved by the human at 15:08) |

Shared beats: **lunch 12:30-13:05** (both screens go dark; on the right the agents keep going and
there is new output when the screen comes back). **17:30** the right side logs off (lock screen with
the clock); the left keeps working until 18:55.

Left-side pain, in order (clock, agent, what happens): 10:45 Claude context limit, re-paste plan and
review · 11:38 Codex context limit · 13:22 Kiro new session, re-paste diff and test list · 13:59
**red test** (old diff pasted) · 14:49 Codex context limit · 15:17 red test · 15:26 Kiro new session ·
16:14 Claude context limit · 16:38 OpenCode context limit · 16:56 red test (old schema pasted) ·
17:45 Claude context limit · 18:03 red test (PR diff missing a commit) · 18:31 Kiro new session ·
18:37 red test (stale Cargo.toml). Totals: **120 pastes, 9 context resets, 5 red test runs.**

Right-side highlights: 09:44 Claude asks Codex to review the search plan · 09:58 Codex replies
(matched to the request) · 11:00 OpenCode finds a query-injection hole and messages **Kiro directly**
with the test to write · 11:30 `ask_user`: approve the `messages_fts` table → notification → one click ·
13:22 (during lunch) OpenCode finds a path escape in file handoff and tells Kiro · 15:08 `ask_user`:
tag v0.2.0 → notification → one click · 15:15 done · 15:20-17:25 the developer writes the 0.2 docs and
a blog draft in Notes · 17:30 logs off. Totals: **0 pastes, 0 resets, 2 decisions, 31 agent messages.**

## 4. What the screen looks like (match the reference video's feel)

Frame (16:9): dark background `#0b0d11`. Two macOS desktops side by side, each 936x702 px, top at
y=90, 14 px from the frame edges. Above each: a label ("Copy-paste" in `#ff9b8f`, "Crosstalk" in
`#ffd27a`) and a status ("Working…" grey → "Shipped · 9h 50m" / "Shipped · 6h 15m" in green). Top
centre: the shared clock in a mono font (09:00 → 19:00) with "TIME-LAPSE ×632" under it. Below each
desktop: the eight tasks as small chips (○ grey → ✓ green) and counters: left "Pastes by me · Context
resets · Red tests", right "Pastes 0 · Resets 0 · Decisions by me". Subtitles: bottom centre, one line,
white on a near-black pill, 38 px. For 1:1 the desktops stack the same way with chips in two rows.

Each desktop (macOS, dark): wallpaper = dusk gradient with soft hills; menu bar with the active app's
name, Wi-Fi, battery and a date/clock "Mon 17 Nov 09:00" that ticks every minute; traffic lights on
every window, the front window has coloured lights, the others grey. Windows overlap like a real desk
and come to the front when used:

- **iTerm2 with a tmux 2x2 split** ("Claude Code", "Codex", "Kiro CLI", "OpenCode" titles). Lines appear
  as the agents print them (JetBrains Mono 8-9 px at this scale, white on `#15171b`; tool calls blue
  `#79c0ff`, Crosstalk sends yellow `#ffd27a`, passing tests green `#7ee787`, pasted blocks `#2d333b`
  background, warnings orange, failures red). Each pane's title shows a context meter "ctx 87%" that
  turns yellow under 40% and red under 15%.
- **Safari** with tabs "Issues · Today" (the task list, ticked live), "PR #130" (a diff page with an
  Approve button) and, on the right side only, "127.0.0.1:7777" (the Crosstalk dashboard: Live view
  with lanes Claude Code / Codex / Kiro / OpenCode / You and arrows for each message, blue request,
  green reply, yellow human; after 15:15 the Threads view: eight threads, each "✓ title · who · n
  messages").
- **VS Code** on the left side (the code grows as features land) and **Notes** on the right side
  (standup notes, an RFC, an email reply, then the 0.2 docs and the blog draft after 15:15).
- **Left only:** a "Clipboard History" panel that fills with entries "#37 Claude Code → Kiro CLI ·
  412 lines · the diff · 11:14"; a blue selection highlight on the text being copied; the cursor
  moving from the source pane to the destination pane for every paste; "[Pasted text #37 +412 lines]"
  in the destination pane.
- **Right only:** macOS notification banners top-right ("Crosstalk · Claude Code asks you / Approve
  adding the messages_fts table? [Approve] [Reject]") with the cursor clicking Approve; a final
  banner "All 8 tasks done · 231 tests pass".
- **Lunch (both):** the desktop dims to a lock screen with the big clock for 12:30-13:05.
- **17:30 (right):** lock screen "Logged off" until the end; a thin green outline on the right desktop
  from 15:15 (shipped). The left gets the same outline at 18:55.
- **58.4-61.6 s:** both desktops dim under a scoreboard: left "9h 50m · 120 pastes · 9 resets · 5 red
  tests" (red), right "6h 15m · 0 pastes · 0 resets · 2 clicks" (green).
- **61.6-66.0 s:** end card: "Crosstalk" (large), "Your agents talk. You make the calls.", "Claude Code ·
  Codex · Kiro · OpenCode · any MCP agent", "github.com/aroraverse/reason-network".

Cursor: a real macOS arrow; left side busy all day (select, ⌘C, move, ⌘V); right side mostly idle,
visible only for the two approvals and some typing in Notes. Camera: no zooms; the time-lapse carries it.

## 5. Timeline (film seconds, 30 fps). Clock = 09:00 + (t − 1.2) × 10.5 min

| Film s | Clock | Left | Right | Narration (= subtitle) |
| --- | --- | --- | --- | --- |
| 0.0-1.2 | 09:00 | both desktops idle, standup notes open | | |
| 0.6 | | | | 1. Monday, nine a.m. One developer, four agents, eight tasks. Two ways to run the day. |
| 2.2 | 09:10 | prompt typed to Claude, agents start | same | |
| 3.5 | 09:23 | Standup ✓ | Standup ✓ | |
| 5.0-6.7 | 09:40-09:58 | pastes begin, clipboard panel opens | #118 ✓; Claude → Codex plan; Codex reply | |
| 7.2 | | | | 2. Left: I'm the clipboard, moving every plan, diff and log by hand. |
| 8.0 | 10:11 | #118 ✓ | | |
| 11.2 | 10:45 | Claude context limit, re-pastes | | |
| 12.3 | | | | 3. Right: the agents talk on Crosstalk and wait for the exact reply. |
| 12.6 | 11:00 | | OpenCode → Kiro direct (test to write) | |
| 15.5-15.7 | 11:30 | | notification, click Approve | |
| 16.3 | 11:38 | Codex context limit | | |
| 16.9 | 11:45 | | #121 ✓ | |
| 17.4 | | | | 4. Every paste burns context. Codex hits its limit; I paste again. |
| 20.4 | 12:21 | #121 ✓ | | |
| 21.2-24.5 | 12:30-13:05 | lunch, dark | lunch, dark (agents continue; #123 ✓ at 23.1) | |
| 21.6 | | | | 5. Lunch. Screens go dark; the agents on the right keep going. |
| 26.0 | | | | 6. OpenCode finds a path escape and tells Kiro what to test. Nothing passes through me. |
| 26.1 | 13:22 | Kiro new session | OpenCode → Kiro direct | |
| 29.6 | 13:59 | **red test** | | |
| 29.7 | 14:00 | | #124 ✓ | |
| 32.8 | 14:32 | #123 ✓ | | 7. Left: a stale diff gets pasted. Tests go red. |
| 34.0-35.2 | 14:45-14:57 | Codex context limit (34.4) | #126 ✓, PR #130 approved | |
| 36.2-36.8 | 15:08-15:15 | | notification, click Approve, Release ✓, "Shipped · 6h 15m", green outline | |
| 36.8 | | | | 8. Quarter past three: the Crosstalk side has shipped everything. Two decisions reached me. |
| 37.1-38.0 | 15:17-15:26 | red test, Kiro new session | Notes: docs for 0.2 | |
| 40.9 | 15:58 | #124 ✓ | | |
| 42.4-44.8 | 16:14-16:38 | Claude, then OpenCode context limit | blog draft | |
| 43.9 | | | | 9. Left side: task six. Context limit, again. |
| 46.5-48.5 | 16:56-17:18 | red test, #126 ✓ | | |
| 49.3 | | | | 10. Half past five. Right side: logged off. Left: still relaying logs. |
| 49.7 | 17:30 | PR #130 pastes (1,900 lines) | lock screen "Logged off" | |
| 51.1-54.1 | 17:45-18:17 | Claude context limit, red test, PR ✓ | | |
| 54.4 | | | | 11. Five to seven. Eight tasks done, a hundred and twenty pastes later. |
| 55.5-57.2 | 18:31-18:50 | Kiro new session, red test, Release ✓, "Shipped · 9h 50m" | | |
| 58.4-61.6 | | scoreboard over both desktops | | 12. (59.6) Crosstalk. Your agents talk. You make the calls. |
| 61.6-66.0 | | end card | | |

Narration lines start no earlier than the times above and never overlap (0.3 s gap); the subtitle is
on screen exactly while its line plays. Full word-level timing is in `narration.srt` (attached).

## 6. Voice, music, sound

- Voice: male, Indian English, calm documentary tone, slightly brisk (Microsoft neural voice
  en-IN-Prabhat at +20% rate was used for the attached `voiceover.mp3`; any equivalent is fine).
  Loudness around -16 LUFS integrated for the mix; music ducks 6-8 dB under the voice.
- Music: ambient, minimal, slow pads in C (Cmaj9 · Am9 · Fmaj9 · G6/9), sparse bells, no drums. A soft
  lift when the right side ships (36.8 s), a resolve on the scoreboard, fade by 66 s. `music.mp3` attached.
- Sound: a soft key tap for every paste (panned left), a quiet two-note chime for each notification and a
  click for each approval (panned right), a low thud on every context reset, a short buzz on every red
  test, a tiny tick every 0.5 s while the time-lapse runs (very low), keyboard typing at 09:10.

## 7. Acceptance checklist

- [ ] Under 70 s; 1920x1080 and 1080x1080; 30 fps; subtitles burned in and identical to the narration.
- [ ] The clock runs 09:00 → 19:00 continuously; nothing on the left jumps ahead of the right's events.
- [ ] Both counters end exactly at: left 120 / 9 / 5, right 0 / 0 / 2. Eight ticks per side.
- [ ] Every event in section 5 is visible at its time (a viewer pausing should find it).
- [ ] No face, no watermark, no "simulated" label, no secrets, emails or personal paths.
- [ ] Realism: overlapping windows, focus changes, lunch dark screen, 17:30 logoff, clipboard panel,
      context meters, notification banners, cursor on the left always busy.

## 8. Files to attach (all in `demo/showcase/video-kit/` unless noted)

| File | What it is for |
| --- | --- |
| `VIDEO_BRIEF.md` (this file, at `demo/showcase/`) | the whole spec |
| `session.json` + `day.py` | every line each agent prints, every paste/reset/red test with clock and film times, the Notes text, the task list, the narration |
| `narration.txt`, `narration.srt` | the script and the subtitle timing |
| `voiceover.mp3`, `vo/01-12.mp3` | the narration placed on the film timeline, and each line separately |
| `music.mp3`, `score.mp3` | the music alone, and music + sound effects mixed as used |
| `sfx/` (`key.mp3`, `click.mp3`, `typing.mp3`) | the effects used |
| `crosstalk-day-16x9.mp4`, `crosstalk-day-1x1.mp4` | the reference cut built from this brief: match its timing and content, improve its realism |
| `stills/` | frames from the reference cut at the key moments, plus two real dashboard screenshots (Live, Approvals) for the exact Crosstalk UI |
| `fonts/HostGrotesk.woff2`, `fonts/AtkinsonMono.woff2` | subtitle/label font and the clock font (UI text uses Inter and JetBrains Mono) |
| `README.md`, `PRODUCT.md`, `STATUS.md` (repo root) | what Crosstalk is and what is verified, for anyone writing extra copy |
| the style reference video (yours) | the on-screen feel to match: a full busy desktop, sped up |
