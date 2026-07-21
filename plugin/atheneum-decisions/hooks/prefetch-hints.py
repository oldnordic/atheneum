#!/usr/bin/env python3
"""prefetch-hints.py -- UserPromptSubmit hook.

Runs memory-prefetch-hints against the submitted prompt and the live session
ID, injecting ranked Memory candidates (BM25 + TF-IDF + recency + session
continuity + optional trajectory bonus) as additionalContext for this turn.
Ports the same mechanism the Hermes atheneum plugin uses, for Claude Code.

Non-blocking by construction: any failure (binary missing, no DB, empty
query, no candidates) exits 0 with no stdout -- never blocks the prompt.

Cross-platform (stdlib only) -- no shell-specific syntax.

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB path),
ATHENEUM_TRAJECTORY_PATH (optional; enables trajectory-graph lookup if set).
Stdin (JSON): prompt, session_id, cwd, ... (UserPromptSubmit schema).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

QUERY_MAX_CHARS = 500
MAX_CONTEXT_CHARS = 4000


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
        return 0

    query = str(hook_input.get("prompt") or "").strip()
    if not query:
        return 0

    binary = _resolve_binary("memory-prefetch-hints", "ATHENEUM_PREFETCH_BIN")
    if binary is None:
        return 0

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".magellan" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    args = [
        binary,
        db,
        "--query",
        query[:QUERY_MAX_CHARS],
        "--k",
        "5",
        "--max-tokens",
        "500",
    ]

    session_id = str(hook_input.get("session_id") or "").strip()
    if session_id:
        args.extend(["--session-id", session_id])

    trajectory_path = os.environ.get("ATHENEUM_TRAJECTORY_PATH", "").strip()
    if trajectory_path and Path(trajectory_path).is_file():
        args.extend(["--trajectory", trajectory_path, "--trajectory-query", "1.0"])

    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=10)
    except Exception:
        return 0

    if result.returncode != 0:
        return 0

    try:
        data = json.loads(result.stdout)
    except Exception:
        return 0

    candidates = data.get("candidates") or []
    candidates = [c for c in candidates if c.get("kind") != "empty"]
    if not candidates:
        return 0

    lines = ["## Atheneum Prefetch Hints"]
    for item in candidates:
        name = str(item.get("name") or item.get("kind") or "")
        score = item.get("score")
        prefix = f"[score={score:.2f}] " if isinstance(score, (int, float)) else ""
        lines.append(f"- {prefix}{name}")

    context = "\n".join(lines)[:MAX_CONTEXT_CHARS]

    output = {
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": context,
        }
    }
    print(json.dumps(output))
    return 0


if __name__ == "__main__":
    sys.exit(main())
