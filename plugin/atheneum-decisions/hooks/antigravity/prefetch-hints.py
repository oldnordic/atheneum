#!/usr/bin/env python3
"""prefetch-hints.py -- Antigravity PreInvocation hook for UserPromptSubmit.

Reads the conversation transcript to find the latest USER_INPUT,
runs memory-prefetch-hints against it, and injects ranked Memory candidates
as an ephemeral message.
Uses a lockfile keyed by conversationId and stepIndex to avoid repeating hints.
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

def get_latest_user_prompt(transcript_path):
    prompt = ""
    step_idx = -1
    if not transcript_path or not Path(transcript_path).is_file():
        return prompt, step_idx
    try:
        with open(transcript_path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line: continue
                item = json.loads(line)
                if item.get("type") == "USER_INPUT":
                    prompt = item.get("content", "")
                    step_idx = item.get("step_index", -1)
    except Exception:
        pass
    return prompt, step_idx

def main():
    try:
        hook_input = json.load(sys.stdin)
    except Exception:
        return 0

    transcript_path = hook_input.get("transcriptPath")
    conversation_id = hook_input.get("conversationId")
    if not transcript_path or not conversation_id:
        return 0

    query, step_idx = get_latest_user_prompt(transcript_path)
    if not query or step_idx < 0:
        return 0

    lock_file = Path(f"/tmp/atheneum_prefetch_{conversation_id}_{step_idx}.lock")
    if lock_file.exists():
        return 0

    binary = _resolve_binary("memory-prefetch-hints", "ATHENEUM_PREFETCH_BIN")
    if binary is None:
        return 0

    db = os.environ.get("ATHENEUM_DB", "").strip()
    if not db:
        db = str(Path.home() / ".hermes" / "atheneum" / "atheneum.db")
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
    
    args.extend(["--session-id", conversation_id])

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
    
    lock_file.touch()

    output = {
        "injectSteps": [
            {
                "ephemeralMessage": context
            }
        ]
    }
    print(json.dumps(output))
    return 0

if __name__ == "__main__":
    sys.exit(main())
