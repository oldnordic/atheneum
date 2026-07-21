#!/usr/bin/env python3
"""decision-gate.py -- Stop hook (non-blocking).

If a session did real work (>=1 tool_call event) but recorded zero Decision
rows for its session_id, remind the operator to record decisions. This is
the Phase 5 soft-warn gate from CHAT_DECISION_PLAN.md. It never blocks the
session and never fails it: any error (envoy down, DB missing, atheneum not
on PATH, bad JSON) makes it exit 0 silently. The reminder goes to stderr so
Claude Code surfaces it without affecting the transcript.

Cross-platform by construction (stdlib only, no shell-specific syntax) --
runs identically on Linux, macOS, and Windows, replacing the earlier
fish-only implementation.

Env: CLAUDE_CODE_SESSION_ID (set by Claude Code for hooks), ATHENEUM_DB
(optional; defaults to the live atheneum DB path).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def _run_atheneum(args):
    """Run an atheneum CLI subcommand, return parsed JSON dict or None on any failure."""
    try:
        result = subprocess.run(
            ["atheneum", *args],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except Exception:
        return None
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout)
    except Exception:
        return None


def main():
    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".magellan" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    session_id = os.environ.get("CLAUDE_CODE_SESSION_ID", "").strip()
    if not session_id:
        return 0

    if shutil.which("atheneum") is None:
        return 0

    decisions = _run_atheneum(
        ["discoveries-recent", db, "--session", session_id, "--type", "Decision", "--limit", "1"]
    )
    if decisions is not None and len(decisions.get("discoveries", [])) > 0:
        return 0

    events = _run_atheneum(
        ["events-recent", db, "--session", session_id, "--type", "tool_call", "--limit", "1"]
    )
    if events is None or len(events.get("events", [])) == 0:
        return 0

    print(
        f"⚠ No decisions recorded for session {session_id}, though it made tool calls. "
        "If you made architectural choices, record one with /decision <target> <chosen> "
        "[rationale] or the record-decision skill.",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
