#!/usr/bin/env fish
# decision-gate.fish — Stop hook (non-blocking).
#
# If a session did real work (>=1 tool_call event) but recorded zero Decision
# rows for its session_id, remind the operator to record decisions. This is the
# Phase 5 soft-warn gate from CHAT_DECISION_PLAN.md. It never blocks the
# session and never fails it: any error (envoy down, DB missing, atheneum not
# on PATH, bad JSON) makes it exit 0 silently. The reminder goes to stderr so
# Claude Code surfaces it without affecting the transcript.
#
# Env: CLAUDE_CODE_SESSION_ID (set by Claude Code for hooks), ATHENEUM_DB
# (optional; defaults to the live atheneum DB path).

set -l DB "$ATHENEUM_DB"
test -z "$DB"; and set DB "$HOME/.magellan/atheneum/atheneum.db"
test -f "$DB"; or exit 0

set -l SID "$CLAUDE_CODE_SESSION_ID"
test -n "$SID"; or exit 0

command -v atheneum >/dev/null 2>&1; or exit 0
command -v python3 >/dev/null 2>&1; or exit 0

# Any Decision rows for this session (any source)?
set -l DEC_JSON (atheneum discoveries-recent "$DB" --session "$SID" --type Decision --limit 1 2>/dev/null)
set -l DEC (echo "$DEC_JSON" | python3 -c "import sys,json
try: print(len(json.load(sys.stdin).get('discoveries', [])))
except Exception: print(0)" 2>/dev/null)
test "$DEC" -gt 0 2>/dev/null; and exit 0

# Did the session do real work (tool_call events)?
set -l EV_JSON (atheneum events-recent "$DB" --session "$SID" --type tool_call --limit 1 2>/dev/null)
set -l EV (echo "$EV_JSON" | python3 -c "import sys,json
try: print(len(json.load(sys.stdin).get('events', [])))
except Exception: print(0)" 2>/dev/null)
test "$EV" -gt 0 2>/dev/null; or exit 0

echo "⚠ No decisions recorded for session $SID, though it made tool calls. If you made architectural choices, record one with /decision <target> <chosen> [rationale] or the record-decision skill." >&2
exit 0