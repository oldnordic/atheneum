#!/usr/bin/env python3
"""decision-gate.py -- Kimi Code CLI Stop hook (atheneum set, non-blocking).

If a session did real work (>=1 tool_call event) but recorded zero Decision
rows for its session_id, print a reminder to record decisions (Phase 5
soft-warn gate from CHAT_DECISION_PLAN.md).

Kimi variant of hooks/decision-gate.py (Claude Code set). Differences: the
reminder goes to STDOUT (Kimi may append hook stdout to context; stderr is
only surfaced as a block reason on exit code 2, and this hook never blocks),
and the session_id comes from the Kimi base payload (session_id), with
generic env fallbacks.

Non-blocking by construction: any error (envoy down, DB missing, atheneum
not on PATH, bad JSON) makes it exit 0 silently.

Cross-platform (stdlib only) -- no shell-specific syntax.

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB paths),
ATHENEUM_BIN (optional; atheneum CLI override).
Stdin (JSON): hook_event_name, session_id, cwd, ... (Kimi Stop payload).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

DB_CANDIDATES = (
    str(Path.home() / ".magellan" / "atheneum" / "atheneum.db"),
    str(Path.home() / ".hermes" / "atheneum" / "atheneum.db"),
)

SESSION_ID_ENVS = ("KIMI_CODE_SESSION_ID", "CLAUDE_CODE_SESSION_ID", "GROUNDED_AGENT_ID")


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


def _resolve_db():
    override = os.environ.get("ATHENEUM_DB", "").strip()
    if override and Path(override).is_file():
        return override
    for candidate in DB_CANDIDATES:
        if Path(candidate).is_file():
            return candidate
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

    db = _resolve_db()
    if db is None:
        return 0

    session_id = str(hook_input.get("session_id") or "").strip()
    if not session_id:
        for env in SESSION_ID_ENVS:
            session_id = os.environ.get(env, "").strip()
            if session_id:
                break
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

    # Stdout, not stderr: Kimi may append hook stdout to context; stderr is
    # only shown when blocking (exit 2), which this hook never does.
    print(
        f"⚠ No decisions recorded for session {session_id}, though it made tool calls. "
        "If you made architectural choices, record one with /atheneum-decisions:decision "
        "<target> <chosen> [rationale] or the record-decision skill."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
