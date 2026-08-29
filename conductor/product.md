# Product Definition: Atheneum

## Overview
Atheneum is an Agent Coordination Graph Database, Memory System, Decision Threader, and Task Management platform designed for autonomous multi-agent software engineering.

## Target Audience
- Autonomous coding agents (AGY, Kimi, Claude, Codex, Hermes) collaborating on complex polyglot software repositories.
- Human supervisors directing multi-agent swarms.

## Core Value Propositions
1. **Persistent Knowledge & Graph Traversal**: Relational graph storage (`sqlitegraph`) of entities, topics, documents, and discoveries with BFS traversal and topological subgraphs.
2. **Decision Tracking & Lineage**: Persisting decision rationales, alternatives considered, and lineage threads across agent sessions.
3. **Multi-Agent Coordination & Kanban**: Task lifecycle (`TODO`, `IN_PROGRESS`, `DONE`, `ARCHIVED`), handoff protocols, and cross-project routing (`meta.db`).
4. **Token-Constrained Recall**: Micro-packet serialization (`session-digest --tokens 500`) to bootstrap agent context without context-window pollution.
5. **Grounded Memory Invalidation**: Pinned AST symbol and receipt hashes to automatically invalidate stale facts when code diverges.
