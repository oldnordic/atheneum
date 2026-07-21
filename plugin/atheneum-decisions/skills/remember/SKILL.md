---
name: remember
description: Use when you learn a durable fact worth persisting across sessions that isn't a Decision (project state, a bug's root cause, a user preference, an operational gotcha, a config that keeps tripping people up). Records it into atheneum via memory-store, keyed so re-running with the same key updates the fact in place instead of duplicating it. Skip for anything trivial, obvious, or already documented in the codebase.
---

# Remember

`atheneum memory-store` is upsert-by-key: it looks up an existing row by
`(key, scope, project_id)` and updates it in place (preserving the original
`created_at`) instead of inserting a duplicate. This makes it safe to
re-run with the same key whenever a fact changes -- there is no separate
dedup flag to remember.

Use this for facts that are **not** architectural decisions (those go
through the `record-decision` skill / `/decision` command instead):
project state ("the migration to X is 60% done"), a root cause worth not
rediscovering, a user preference, an operational gotcha (a flaky test, a
footgun in a script), or a config value that's easy to get wrong.

Skip anything trivial, obvious from reading the code, or already covered
by existing docs -- over-recording is as much noise as under-recording.

## How to record

```bash
DB="${ATHENEUM_DB:-$HOME/.magellan/atheneum/atheneum.db}"
atheneum memory-store "$DB" "<key>" "<content>" --scope memory --project "<project>"
```

- `<key>` -- a short, stable, kebab-case slug for the fact (e.g.
  `flaky-test-retry-behavior`, `migration-progress`). Reuse the same key
  when the fact changes; the CLI upserts rather than duplicating.
- `<content>` -- the fact itself, as a self-contained sentence or two.
  Someone reading only this string in a future session should understand
  it without needing the current conversation for context.
- `--project` -- the project this fact belongs to (usually the repo name).
  Omit only for facts that are genuinely cross-project.
- `--confidence <0.0-1.0>` -- optional, defaults to a reasonable value.
  Lower it for a hunch, raise it for something verified.

After recording, continue with the task. Do not ask the user to confirm and
do not summarize the recording unless it failed.
