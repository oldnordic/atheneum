#!/usr/bin/env python3
"""feedback-capture.py -- Stop hook (non-blocking).

Cheap, deterministic scan of the just-finished exchange for correction or
confirmation signals ("no, don't do that" / "yes exactly, keep doing that").
On a hit, writes a low-confidence `feedback`-scoped memory directly via
`atheneum memory-store` -- no LLM call, no human review gate. Safety net
against over-capture is `atheneum dream --auto-merge` (run periodically,
separately) which deduplicates/flags-stale/merges noisy or repeated entries;
this hook does not need to be conservative because that cleanup already
exists.

Idempotent by construction: the memory key is a hash of the triggering user
message, and `memory-store` upserts by (key, scope, project) -- re-firing on
an unchanged exchange just rewrites the same row, never duplicates.

Non-blocking: any failure (no transcript, no DB, no atheneum binary, no
signal match) exits 0 with no output.

Cross-platform (stdlib only) -- no shell-specific syntax.

Env: ATHENEUM_DB (optional; defaults to the live atheneum DB path),
ATHENEUM_BIN (optional explicit path override, see other hooks in this
plugin for why).
Stdin (JSON): session_id, transcript_path, cwd, ... (Stop input schema).
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

CORRECTION_PATTERNS = [
    r"\bno[,.]?\s+(don'?t|stop|please don'?t)\b",
    r"\bstop doing\b",
    r"\bthat'?s not what i (meant|asked|wanted)\b",
    r"\bthat'?s wrong\b",
    r"\bwhy did you\b",
    r"\bi didn'?t ask for\b",
    r"\bnever do\b",
]

CONFIRMATION_PATTERNS = [
    r"\b(yes|yeah|yep),?\s*exactly\b",
    r"\bkeep doing that\b",
    r"\bperfect,? (keep|that'?s)\b",
    r"\bgood catch\b",
    r"\b(that'?s|was) the right (call|approach)\b",
]

CONTENT_MAX_CHARS = 400


def _resolve_binary(name, env_var):
    """Env var override -> PATH lookup -> ~/.local/bin fallback."""
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


def _message_text(entry):
    content = entry.get("message", {}).get("content", "")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = [
            c.get("text", "")
            for c in content
            if isinstance(c, dict) and c.get("type") == "text"
        ]
        return " ".join(parts)
    return ""


def _last_exchange(transcript_path):
    """Return (last_user_text, last_assistant_text_before_it) or (None, None)."""
    last_user = None
    last_assistant_before_user = None
    try:
        with open(transcript_path, "r") as f:
            lines = f.readlines()
    except Exception:
        return None, None

    pending_assistant = ""
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            entry = json.loads(line)
        except Exception:
            continue
        etype = entry.get("type")
        if etype == "assistant":
            text = _message_text(entry)
            if text:
                pending_assistant = text
        elif etype == "user":
            text = _message_text(entry)
            if text:
                last_user = text
                last_assistant_before_user = pending_assistant

    return last_user, last_assistant_before_user


def _classify(user_text):
    lowered = user_text.lower()
    for pat in CORRECTION_PATTERNS:
        if re.search(pat, lowered):
            return "correction"
    for pat in CONFIRMATION_PATTERNS:
        if re.search(pat, lowered):
            return "confirmation"
    return None


def main():
    try:
        hook_input = json.load(sys.stdin)
    except Exception:
        return 0

    transcript_path = hook_input.get("transcript_path", "")
    if not transcript_path or not Path(transcript_path).is_file():
        return 0

    user_text, assistant_text = _last_exchange(transcript_path)
    if not user_text:
        return 0

    kind = _classify(user_text)
    if kind is None:
        return 0

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".hermes" / "atheneum" / "atheneum.db")
    if not Path(db).is_file():
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    project_dir = os.environ.get("CLAUDE_PROJECT_DIR", "").strip() or hook_input.get("cwd", "")
    project = Path(project_dir).name if project_dir else "unknown"

    content = (
        f"[{kind}] User: \"{user_text[:CONTENT_MAX_CHARS]}\" "
        f"-- context, assistant had just done: \"{assistant_text[:CONTENT_MAX_CHARS]}\""
    )
    key = "feedback-auto-" + hashlib.sha1(user_text.encode("utf-8")).hexdigest()[:12]

    try:
        subprocess.run(
            [
                atheneum_bin,
                "memory-store",
                db,
                key,
                content,
                "--scope",
                "feedback",
                "--project",
                project,
                "--confidence",
                "0.3",
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except Exception:
        return 0

    return 0


if __name__ == "__main__":
    sys.exit(main())
