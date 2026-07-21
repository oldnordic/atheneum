---
name: record-decision
description: Use when you choose between two or more implementation approaches, make an architectural tradeoff, or pick one option over alternatives — records the decision into atheneum (source=skill) so the decision chain persists across sessions. Non-blocking; skip when no real choice was made or alternatives were considered.
---

# Record Decision

When you make a genuine decision — choosing between approaches, picking an option
over alternatives, committing to an architectural tradeoff — record it so the next
session can see *why*, not just *what*. Atheneum holds the decision chain; this
skill is the highest-fidelity layer (the model records the decision as it makes
one), complementing the transcript watcher (deterministic, post-hoc) and the LLM
backfiller.

Record **only real choices**. Do not record routine tool picks, obvious fixes,
typo corrections, or anything where no alternatives were seriously considered.
Over-recording is noise; under-recording loses the chain. When unsure, skip.

## What to record

- `target` — short slug for the decision subject (e.g. `storage-engine`,
  `auth-strategy`, `migration-tooling`).
- `chosen` — the option you picked, as a short phrase.
- `alternatives` — the options you rejected (array of short phrases).
- `rationale` — one or two sentences on why `chosen` won.

## How to record

Run the bash block below, substituting your values for `<TARGET>`, `<CHOSEN>`,
the `<ALT...>` entries, and `<RATIONALE>`. Keep `source` as `"skill"`.
`$CLAUDE_CODE_SESSION_ID` is set by Claude Code; `$ATHENEUM_DB` is the live
atheneum DB (falls back to the default path).

```bash
DB="${ATHENEUM_DB:-$HOME/.hermes/atheneum/atheneum.db}"
T=$(mktemp --suffix=.json)
cat > "$T" <<JSON
{"source":"skill","chosen":"<CHOSEN>","alternatives":["<ALT1>","<ALT2>"],"rationale":"<RATIONALE>","target":"<TARGET>"}
JSON
atheneum store-discovery "$DB" claude Decision "<TARGET>" "$T" --session "$CLAUDE_CODE_SESSION_ID" --dedup
rm -f "$T"
```

`--dedup` skips the insert when the same `(session_id, target, source=skill,
chosen)` decision was already recorded, so re-firing on the same choice does
**not** double-capture. The output JSON carries `"deduped": true` when it
skipped and `"discovery_id"` set when it stored. `--force` would bypass dedup;
do not use it here.

After recording, continue with the task. Do not ask the user to confirm and do
not summarize the recording unless it failed.