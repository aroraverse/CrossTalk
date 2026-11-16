#!/usr/bin/env bash
# Crosstalk showcase director for macOS (tmux, four real agents, one real task).
#
# Opens a tmux session "crosstalk" with four panes, each running a real agent
# in demo/showcase/refund-service, types each agent's prompt at the right
# moment, and waits on the Crosstalk database (read-only) between steps, so the
# run is real and repeatable. The human approval stays a real click in the
# dashboard: the script pauses until the approval is answered.
#
# Usage:  bash demo/showcase/mac/director.sh          (from the repository root)
# Needs:  tmux, sqlite3 (both ship with Homebrew / macOS), the four agent CLIs,
#         `crosstalk daemon-start` already running, `crosstalk setup --agent all` done.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
WORK="$ROOT/demo/showcase/refund-service"
PROMPTS="$ROOT/demo/showcase/prompts"
DB="${CROSSTALK_DB:-$HOME/.local/share/crosstalk/crosstalk.db}"
[ -f "$DB" ] || DB="$HOME/Library/Application Support/crosstalk/crosstalk.db"
[ -f "$DB" ] || { echo "Crosstalk database not found; start the daemon first (crosstalk daemon-start)"; exit 1; }

q() { sqlite3 -readonly "$DB" "$1"; }
wait_for() {  # wait_for "<description>" "<sql returning a count>" <min>
  printf '  waiting: %s' "$1"
  until [ "$(q "$2")" -ge "$3" ]; do sleep 2; printf '.'; done
  echo ' ok'
}
type_prompt() {  # type_prompt <pane> <file>: types the prompt as one line, then Enter
  local text; text="$(tr '\n' ' ' < "$2")"
  tmux send-keys -t "crosstalk:0.$1" -l "$text"
  sleep 0.5
  tmux send-keys -t "crosstalk:0.$1" Enter
}
since="$(date +%s)"
msgs_to() { echo "select count(*) from messages where recipient_key like '$1-%' and created_at >= $since"; }

# Fresh copy of the project for every take.
git -C "$ROOT" checkout -- demo/showcase/refund-service 2>/dev/null || true

tmux kill-session -t crosstalk 2>/dev/null || true
tmux new-session -d -s crosstalk -x 240 -y 64 -c "$WORK"
tmux split-window -h -t crosstalk:0 -c "$WORK"
tmux split-window -v -t crosstalk:0.0 -c "$WORK"
tmux split-window -v -t crosstalk:0.2 -c "$WORK"
tmux select-layout -t crosstalk:0 tiled
tmux set -t crosstalk pane-border-status top
tmux set -t crosstalk pane-border-format " #{pane_title} "
for p in 0 1 2 3; do tmux select-pane -t "crosstalk:0.$p" -T "$(echo "Claude Code|Codex|Kiro CLI|OpenCode" | cut -d'|' -f$((p+1)))"; done

tmux send-keys -t crosstalk:0.0 "claude" Enter
tmux send-keys -t crosstalk:0.1 "codex" Enter
tmux send-keys -t crosstalk:0.2 "kiro-cli chat" Enter
tmux send-keys -t crosstalk:0.3 "opencode" Enter
echo "Attach in iTerm2 with:  tmux attach -t crosstalk   (start the screen recording now)"
read -r -p "Press Enter when all four agents show their prompt and recording has started... " _

echo "1. Claude: plan, then ask Codex for a review"
type_prompt 0 "$PROMPTS/1-claude-lead.md"
wait_for "Claude's plan reaches Codex" "$(msgs_to codex)" 1

echo "2. Codex reviews"
type_prompt 1 "$PROMPTS/2-codex-review.md"
wait_for "an approval question for the human" "select count(*) from questions where created_at >= $since" 1

echo "3. THE HUMAN: open http://127.0.0.1:7777/#/approvals and click Approve (film this in real time)"
wait_for "the human's answer" "select count(*) from questions where created_at >= $since and status = 'answered'" 1

echo "4. Claude implements, then asks Kiro (tests) and OpenCode (security)"
wait_for "requests to Kiro and OpenCode" "select (select count(*) from messages where recipient_key like 'kiro-%' and created_at >= $since) >= 1 and (select count(*) from messages where recipient_key like 'opencode-%' and created_at >= $since) >= 1" 1

echo "5. OpenCode: security review, tells Kiro directly"
type_prompt 3 "$PROMPTS/3-opencode-security.md"
wait_for "OpenCode's note to Kiro" "$(msgs_to kiro)" 2

echo "6. Kiro: writes the tests and runs them"
type_prompt 2 "$PROMPTS/4-kiro-tests.md"
wait_for "Kiro's replies" "select count(*) from messages where sender_key like 'kiro-%' and created_at >= $since" 2

echo "7. Let Claude finish its summary, then show http://127.0.0.1:7777/#/live full screen for 10 s. Done."
