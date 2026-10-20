#!/usr/bin/env python3
"""Structural banned-pattern linter for atheneum.

Enforces the same gate used in grounded-kernel:
- no todo/fixme/hack style deferred-work markers in production comments
- no Rust todo!/unimplemented! macros in production code
- no undocumented Rust warning suppressions
- no obvious hardcoded stub returns in production paths
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

EXCLUDE_PARTS = frozenset(
    {
        ".git",
        ".pytest_cache",
        ".venv",
        "__pycache__",
        "target",
        "tests",
    }
)

PATTERN_RULE_MODULES = frozenset(
    {
        "audit.py",
        "audit_security.py",
        "detectors.py",
        "gate.py",
        "gitops.py",
        "invariants.py",
        "mutator.py",
        "qa.py",
        "rules.py",
        "banned_patterns_lint.py",
    }
)

COMMENT_BANNED_RE = re.compile(
    r"(?://|#)\s*(TODO|FIXME|HACK|XXX|todo|fixme|for now|will fix|temporary hack|will fix later)",
    re.IGNORECASE,
)
RUST_TODO_RE = re.compile(r"\b(?:todo|unimplemented)!")
RUST_ALLOW_RE = re.compile(r"#\[allow\(")
RUST_ALLOW_REASON_RE = re.compile(r"reason\s*=")
RUST_ALLOW_DEAD_CODE_RE = re.compile(r"#\[allow\(dead_code\)\]")
STUB_RETURN_RE = re.compile(
    r"return\s+(?:true|false|Ok\((?:\(\)|0|\"\")\))\s*;\s*(?://\s*stub\b|#\s*stub\b)",
    re.IGNORECASE,
)


def find_source_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for ext in ("*.py", "*.rs"):
        for path in root.rglob(ext):
            rel = path.relative_to(root)
            if any(part in EXCLUDE_PARTS or part.startswith(".") for part in rel.parts):
                continue
            files.append(path)
    return sorted(files)


def allow_attr_has_reason(lines: list[str], index: int) -> bool:
    line = lines[index]
    if RUST_ALLOW_REASON_RE.search(line):
        return True

    previous = lines[index - 1].strip() if index > 0 else ""
    if previous.startswith("//") and "reason:" in previous.lower():
        return True

    if "]" in line:
        return False

    for next_line in lines[index + 1 : min(index + 5, len(lines))]:
        if RUST_ALLOW_REASON_RE.search(next_line):
            return True
        if "]" in next_line:
            return False
    return False


def scan_file(path: Path, root: Path) -> list[dict[str, object]]:
    rel = str(path.relative_to(root))
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return [{"file": rel, "line": None, "pattern": "<unreadable>", "text": str(exc)}]

    hits: list[dict[str, object]] = []
    lines = text.splitlines()
    is_rule_module = path.name in PATTERN_RULE_MODULES
    is_rust = path.suffix == ".rs"

    for lineno, line in enumerate(lines, start=1):
        stripped = line.strip()

        if is_rule_module:
            if stripped.startswith(('"', "'", "r'", 'r"', "re.compile(")) or stripped.endswith(
                (",", "])", "})", '"):')
            ):
                continue
            if '"""' in stripped or "'''" in stripped:
                continue

        match = COMMENT_BANNED_RE.search(line)
        if match:
            hits.append(
                {
                    "file": rel,
                    "line": lineno,
                    "pattern": f"Banned comment '{match.group(1)}'",
                    "text": stripped[:120],
                }
            )

        if is_rust:
            if RUST_TODO_RE.search(line):
                hits.append(
                    {
                        "file": rel,
                        "line": lineno,
                        "pattern": "Rust todo!/unimplemented!",
                        "text": stripped[:120],
                    }
                )
            if RUST_ALLOW_DEAD_CODE_RE.search(line):
                hits.append(
                    {
                        "file": rel,
                        "line": lineno,
                        "pattern": "#[allow(dead_code)] (prohibited)",
                        "text": stripped[:120],
                    }
                )
            elif RUST_ALLOW_RE.search(line) and not allow_attr_has_reason(lines, lineno - 1):
                hits.append(
                    {
                        "file": rel,
                        "line": lineno,
                        "pattern": "#[allow(...)] without documented reason",
                        "text": stripped[:120],
                    }
                )
            if STUB_RETURN_RE.search(line):
                hits.append(
                    {
                        "file": rel,
                        "line": lineno,
                        "pattern": "Hardcoded stub return in production path",
                        "text": stripped[:120],
                    }
                )

    return hits


def run_lint(root: Path) -> int:
    files = find_source_files(root)
    all_hits: list[dict[str, object]] = []
    for source_file in files:
        all_hits.extend(scan_file(source_file, root))

    if all_hits:
        print(f"[BANNED-PATTERN LINT] Found {len(all_hits)} violation(s):", file=sys.stderr)
        for hit in all_hits:
            loc = f"{hit['file']}:{hit['line']}" if hit.get("line") else hit["file"]
            print(f"  - {loc} [{hit['pattern']}]: {hit['text']}", file=sys.stderr)
        return 1

    print(f"[BANNED-PATTERN LINT] Scanned {len(files)} production source files - 0 violations.")
    return 0


if __name__ == "__main__":
    sys.exit(run_lint(REPO_ROOT))
