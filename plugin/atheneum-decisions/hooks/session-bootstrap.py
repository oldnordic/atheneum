#!/usr/bin/env python3
"""session-bootstrap.py -- SessionStart hook.

Injects a bounded, ranked session digest (recent tool calls, file writes,
decisions, open tasks, thread anchors) as additionalContext at session start,
so the model is grounded on prior project work before the first turn --
without needing the atheneum-mcp server or a manual session-digest call.

Non-blocking by construction: any failure (no DB, atheneum not on PATH,
empty digest) exits 0 with no stdout, so a missing/misconfigured atheneum
install never affects a session that doesn't use it.

Cross-platform (stdlib only) -- no shell-specific syntax.

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB path).
Stdin (JSON): cwd, session_id, source, ... (SessionStart input schema).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

MAX_TOKENS = 500
MAX_CONTEXT_CHARS = 8000


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


def main():
    try:
        hook_input = json.load(sys.stdin)
    except Exception:
        hook_input = {}

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".magellan" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    project_dir = os.environ.get("CLAUDE_PROJECT_DIR", "").strip() or hook_input.get("cwd", "")
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

    output = {
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": f"## Atheneum Session Digest ({project})\n{digest}",
        }
    }
    print(json.dumps(output))
    return 0


if __name__ == "__main__":
    sys.exit(main())
