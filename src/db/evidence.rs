//! Stage 11e: Evidence-domain SQL tables (v4 migration).
//!
//! Agnostic schema for recording LLM-assisted development metrics.
//! Tables: sessions, commits, test_runs, bench_runs, releases, fix_chains, event_log.
//! Forge is one client — any tool can use these tables.

use anyhow::Result;
use rusqlite::Transaction;

pub fn migrate_v4_evidence(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            session_id       TEXT PRIMARY KEY,
            agent_id         INTEGER NOT NULL REFERENCES agents(id),
            parent_session_id TEXT,
            project          TEXT NOT NULL,
            tool             TEXT NOT NULL,
            trigger          TEXT NOT NULL DEFAULT 'cli',
            model            TEXT,
            started_at       TEXT NOT NULL,
            ended_at         TEXT,
            exit_status      TEXT,
            git_branch       TEXT,
            git_head         TEXT,
            user             TEXT,
            prompt_count     INTEGER DEFAULT 0,
            tool_call_count  INTEGER DEFAULT 0,
            file_write_count INTEGER DEFAULT 0,
            commit_count     INTEGER DEFAULT 0,
            test_run_count   INTEGER DEFAULT 0,
            total_input_tokens  INTEGER DEFAULT 0,
            total_output_tokens INTEGER DEFAULT 0,
            total_cost_usd   REAL DEFAULT 0.0
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project);
        CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
        CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_tool ON sessions(tool);

        ALTER TABLE reasoning_logs ADD COLUMN session_id TEXT REFERENCES sessions(session_id);

        ALTER TABLE tool_calls ADD COLUMN session_id TEXT REFERENCES sessions(session_id);

        CREATE TABLE IF NOT EXISTS commits (
            commit_id       TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(session_id),
            commit_sha      TEXT NOT NULL,
            parent_sha      TEXT,
            message         TEXT NOT NULL,
            author          TEXT NOT NULL,
            timestamp       TEXT NOT NULL,
            files_changed   INTEGER NOT NULL,
            lines_inserted  INTEGER NOT NULL,
            lines_deleted   INTEGER NOT NULL,
            commit_type     TEXT NOT NULL,
            fixes_commit_id TEXT,
            feature_tag     TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_commits_session ON commits(session_id);
        CREATE INDEX IF NOT EXISTS idx_commits_sha ON commits(commit_sha);
        CREATE INDEX IF NOT EXISTS idx_commits_type ON commits(commit_type);
        CREATE INDEX IF NOT EXISTS idx_commits_feature ON commits(feature_tag);
        CREATE INDEX IF NOT EXISTS idx_commits_fixes ON commits(fixes_commit_id);

        CREATE TABLE IF NOT EXISTS test_runs (
            test_run_id     TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(session_id),
            commit_sha      TEXT,
            test_command    TEXT NOT NULL,
            test_suite      TEXT,
            test_name       TEXT NOT NULL,
            result          TEXT NOT NULL,
            duration_ms     INTEGER NOT NULL,
            logs_hash       TEXT,
            logs_summary    TEXT,
            is_regression   INTEGER DEFAULT 0,
            previous_run_id TEXT,
            regression_fixed_by TEXT,
            timestamp       TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_test_runs_session ON test_runs(session_id);
        CREATE INDEX IF NOT EXISTS idx_test_runs_name ON test_runs(test_name);
        CREATE INDEX IF NOT EXISTS idx_test_runs_result ON test_runs(result);
        CREATE INDEX IF NOT EXISTS idx_test_runs_regression ON test_runs(is_regression);
        CREATE INDEX IF NOT EXISTS idx_test_runs_commit ON test_runs(commit_sha);

        CREATE TABLE IF NOT EXISTS bench_runs (
            bench_run_id    TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(session_id),
            commit_sha      TEXT,
            bench_name      TEXT NOT NULL,
            bench_suite     TEXT,
            mean_ns         INTEGER,
            median_ns       INTEGER,
            p95_ns          INTEGER,
            p99_ns          INTEGER,
            stddev_ns       INTEGER,
            iterations      INTEGER,
            throughput      REAL,
            baseline_mean_ns INTEGER,
            regression_pct  REAL,
            is_regression   INTEGER DEFAULT 0,
            timestamp       TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_bench_runs_name ON bench_runs(bench_name);
        CREATE INDEX IF NOT EXISTS idx_bench_runs_regression ON bench_runs(is_regression);

        CREATE TABLE IF NOT EXISTS releases (
            release_id      TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(session_id),
            version_tag     TEXT NOT NULL,
            commit_sha      TEXT NOT NULL,
            previous_tag    TEXT,
            timestamp       TEXT NOT NULL,
            commits_since_last INTEGER,
            features_count  INTEGER,
            fixes_count     INTEGER,
            breaking_changes INTEGER,
            tests_total     INTEGER,
            tests_new       INTEGER,
            tests_passing   INTEGER,
            benches_total   INTEGER,
            bench_regressions INTEGER,
            production_loc  INTEGER,
            test_loc        INTEGER,
            total_loc       INTEGER,
            total_api_cost_usd REAL,
            active_dev_days REAL,
            quality_score   REAL
        );
        CREATE INDEX IF NOT EXISTS idx_releases_tag ON releases(version_tag);

        CREATE TABLE IF NOT EXISTS fix_chains (
            fix_chain_id    TEXT PRIMARY KEY,
            bug_commit_id   TEXT NOT NULL REFERENCES commits(commit_id),
            bug_tool_call_id INTEGER REFERENCES tool_calls(id),
            bug_file_id     TEXT,
            fix_commit_id   TEXT NOT NULL REFERENCES commits(commit_id),
            fix_session_id  TEXT NOT NULL REFERENCES sessions(session_id),
            fix_type        TEXT NOT NULL,
            severity        TEXT NOT NULL,
            cycles_to_fix   INTEGER NOT NULL,
            bug_timestamp   TEXT NOT NULL,
            fix_timestamp   TEXT NOT NULL,
            time_to_fix_ms  INTEGER NOT NULL,
            fix_input_tokens INTEGER,
            fix_output_tokens INTEGER,
            fix_cost_usd   REAL
        );
        CREATE INDEX IF NOT EXISTS idx_fix_chains_bug ON fix_chains(bug_commit_id);
        CREATE INDEX IF NOT EXISTS idx_fix_chains_fix ON fix_chains(fix_commit_id);
        CREATE INDEX IF NOT EXISTS idx_fix_chains_type ON fix_chains(fix_type);

        CREATE TABLE IF NOT EXISTS event_log (
            event_id        INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type      TEXT NOT NULL,
            entity_id       TEXT NOT NULL,
            session_id      TEXT NOT NULL REFERENCES sessions(session_id),
            payload_hash    TEXT NOT NULL,
            payload         TEXT NOT NULL,
            timestamp       TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_event_log_type ON event_log(event_type);
        CREATE INDEX IF NOT EXISTS idx_event_log_session ON event_log(session_id);
        CREATE INDEX IF NOT EXISTS idx_event_log_timestamp ON event_log(timestamp);",
    )?;
    Ok(())
}
