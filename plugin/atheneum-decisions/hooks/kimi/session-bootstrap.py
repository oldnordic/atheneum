#!/usr/bin/env python3
"""session-bootstrap.py -- Kimi Code CLI SessionStart hook (atheneum set).

Prints a bounded, ranked session digest (recent tool calls, file writes,
decisions, open tasks, thread anchors) to stdout at session start. Kimi Code
may append hook stdout to the model's context, so the model is grounded on
prior project work before the first turn -- without needing the
atheneum-mcp server or a manual session-digest call.

Kimi variant of hooks/session-bootstrap.py (Claude Code set). Differences:
plain-text stdout instead of the Claude hookSpecificOutput JSON schema, and
the project dir comes from the stdin payload's `cwd` (Kimi base payload:
hook_event_name, session_id, cwd).

Non-blocking by construction: any failure (no DB, atheneum not on PATH,
empty digest) exits 0 with no stdout, so a missing/misconfigured atheneum
install never affects a session that doesn't use it.

Cross-platform (stdlib only) -- no shell-specific syntax.

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB paths),
ATHENEUM_BIN (optional; atheneum CLI override).
Stdin (JSON): hook_event_name, session_id, cwd, ... (Kimi base payload).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

MAX_TOKENS = 500
MAX_CONTEXT_CHARS = 8000

DB_CANDIDATES = (
    str(Path.home() / ".magellan" / "atheneum" / "atheneum.db"),
    str(Path.home() / ".hermes" / "atheneum" / "atheneum.db"),
)


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


def main():
    try:
        hook_input = json.load(sys.stdin)
    except Exception:
        hook_input = {}

    db = _resolve_db()
    if db is None:
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    project_dir = str(hook_input.get("cwd") or "").strip()
    project = Path(project_dir).name if project_dir else ""
    if not project:
        return 0

    try:
        result = subprocess.run(
            [
                atheneum_bin,
                "session-digest",
                db,
                "--project",
                project,
                "--last",
                "3",
                "--tokens",
                str(MAX_TOKENS),
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except Exception:
        return 0

    digest = result.stdout.strip()
    if result.returncode != 0 or not digest:
        return 0

    digest = digest[:MAX_CONTEXT_CHARS]

    # Kimi may append hook stdout to context; plain text, no JSON schema.
    print(f"## Atheneum Session Digest ({project})\n{digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
