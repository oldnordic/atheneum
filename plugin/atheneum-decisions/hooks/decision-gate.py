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

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB path).
Stdin (JSON): session_id, transcript_path, cwd, ... (Stop input schema).
CLAUDE_CODE_SESSION_ID env var used only as a fallback if stdin lacks
session_id.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def _resolve_binary(name, env_var):
    """Env var override -> PATH lookup -> ~/.local/bin fallback.

    Hook scripts don't always inherit a full login-shell PATH, so PATH
    lookup alone silently no-ops even when the binary is installed.
    """
    override = os.environ.get(env_var, "").strip()
    if override and Path(override).is_file():
        return override
    found = shutil.which(name)
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / name
    if fallback.is_file():
        return str(fallback)
    return None


def _run_atheneum(atheneum_bin, args):
    """Run an atheneum CLI subcommand, return parsed JSON dict or None on any failure."""
    try:
        result = subprocess.run(
            [atheneum_bin, *args],
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
    try:
        hook_input = json.load(sys.stdin)
    except Exception:
        hook_input = {}

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".hermes" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    session_id = str(hook_input.get("session_id") or "").strip()
    if not session_id:
        session_id = os.environ.get("CLAUDE_CODE_SESSION_ID", "").strip()
    if not session_id:
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    decisions = _run_atheneum(
        atheneum_bin,
        ["discoveries-recent", db, "--session", session_id, "--type", "Decision", "--limit", "1"],
    )
    if decisions is not None and len(decisions.get("discoveries", [])) > 0:
        return 0

    events = _run_atheneum(
        atheneum_bin,
        ["events-recent", db, "--session", session_id, "--type", "tool_call", "--limit", "1"],
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
