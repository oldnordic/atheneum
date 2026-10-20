# Git Hooks Architecture

The scripts in this directory (`.githooks/`) are the canonical version-controlled source for repository gates:
- `pre-commit`: gates conductor track status changes (blocks `COMPLETED` unless full check and test suites pass).
- `pre-push`: structural banned-pattern linter preventing stubs, mocks, and suppressed warnings.

## External Active Hooks Relocation
To protect gates against worker-level tampering during automated agent dispatches, the active hooks directory is relocated outside the repository tree to:
`~/.config/agent-gate-hooks/atheneum/`

Installation and sync is managed by `agy-lockdown/install_repo_hooks.sh`:
```bash
/home/feanor/Projects/agy-lockdown/install_repo_hooks.sh --repo .
```
This configures `core.hooksPath` to point to the external path, ensuring in-repo working tree mutations cannot bypass or weaken verification gates.
