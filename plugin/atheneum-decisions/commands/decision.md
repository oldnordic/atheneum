---
allowed-tools: Bash(atheneum store-discovery:*), Bash(mktemp:*), Bash(rm:*), Bash(rm -f:*)
description: Manually record a decision into atheneum (source=skill) — fallback for when the record-decision skill did not auto-fire.
argument-hint: <target> <chosen> [rationale...]
disable-model-invocation: false
---

# /decision

`$ARGUMENTS` is `<target> <chosen> [rationale...]`.

Parse it:
- `target` — the first token.
- `chosen` — the second token.
- `rationale` — all remaining tokens joined by spaces. If absent, use the
  empty string.

If `target` or `chosen` is missing, tell the user the usage
(`/decision <target> <chosen> [rationale...]`) and stop — do not store anything.

Otherwise run this bash block, substituting the parsed values. Keep `source` as
`"skill"`. `$CLAUDE_CODE_SESSION_ID` is set by Claude Code; `$ATHENEUM_DB` is the
live atheneum DB (falls back to the default path).

```bash
DB="${ATHENEUM_DB:-$HOME/.hermes/atheneum/atheneum.db}"
T=$(mktemp --suffix=.json)
cat > "$T" <<JSON
{"source":"skill","chosen":"<CHOSEN>","alternatives":[],"rationale":"<RATIONALE>","target":"<TARGET>"}
JSON
atheneum store-discovery "$DB" claude Decision "<TARGET>" "$T" --session "$CLAUDE_CODE_SESSION_ID" --dedup
rm -f "$T"
```

Report the JSON result from `atheneum store-discovery` to the user in one line:
the `discovery_id` (or `deduped: true` if it skipped an already-recorded
decision). Do not store duplicates on purpose; if `deduped` is true, tell the
user the decision was already recorded and stop.