#!/usr/bin/env python3
"""session-bootstrap.py -- Antigravity PreInvocation hook for Session Bootstrap.

Injects a bounded, ranked session digest at the start of a conversation.
Uses a lockfile to ensure it only injects once per conversationId.
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
        return 0

    conversation_id = hook_input.get("conversationId")
    if not conversation_id:
        return 0

    lock_file = Path(f"/tmp/atheneum_session_bootstrap_{conversation_id}.lock")
    if lock_file.exists():
        return 0

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".hermes" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    workspace_paths = hook_input.get("workspacePaths", [])
    project = ""
    if workspace_paths:
        project = Path(workspace_paths[0]).name

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
    
    # Touch lockfile
    lock_file.touch()

    output = {
        "injectSteps": [
            {
                "ephemeralMessage": f"## Atheneum Session Digest ({project})\n{digest}"
            }
        ]
    }
    print(json.dumps(output))
    return 0

if __name__ == "__main__":
    sys.exit(main())
