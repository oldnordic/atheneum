#!/usr/bin/env python3
"""decision-gate.py -- Antigravity Stop hook for Decision Accountability.

If a session did real work (tool calls) but recorded zero Decision rows
for its conversationId, remind the agent to record decisions by soft-blocking
the stop once. Subsequent stop attempts will be allowed.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

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

def _run_atheneum(atheneum_bin, args):
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

def has_tool_calls(transcript_path):
    if not transcript_path or not Path(transcript_path).is_file():
        return False
    try:
        with open(transcript_path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line: continue
                item = json.loads(line)
                if item.get("tool_calls") and len(item["tool_calls"]) > 0:
                    return True
    except Exception:
        pass
    return False

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

    conversation_id = hook_input.get("conversationId")
    if not conversation_id:
        return 0

    lock_file = Path(f"/tmp/atheneum_decision_warned_{conversation_id}.lock")
    if lock_file.exists():
        # Already warned once, let it stop
        return 0

    transcript_path = hook_input.get("transcriptPath")
    if not has_tool_calls(transcript_path):
        return 0

    atheneum_bin = _resolve_binary("atheneum", "ATHENEUM_BIN")
    if atheneum_bin is None:
        return 0

    decisions = _run_atheneum(
        atheneum_bin,
        ["discoveries-recent", db, "--session", conversation_id, "--type", "Decision", "--limit", "1"],
    )
    if decisions is not None and len(decisions.get("discoveries", [])) > 0:
        return 0

    # Soft-block: Touch lock file so we don't block next time
    lock_file.touch()

    output = {
        "decision": "continue",
        "reason": (
            "⚠ No decisions recorded for this session, though it made tool calls. "
            "If you made architectural choices, record one with the 'record-decision' skill "
            "before stopping. If no decisions were made, simply stop again to exit."
        )
    }
    print(json.dumps(output))
    return 0

if __name__ == "__main__":
    sys.exit(main())
