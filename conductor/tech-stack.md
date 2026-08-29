# Tech Stack: Atheneum

## Language & Toolchain
- **Language**: Rust (Edition 2021, MSRV 1.82)
- **Workspace Architecture**: Cargo workspace
  - `crates/atheneum`: Core database engine, graph logic, and CLI.
  - `crates/atheneum-mcp`: Model Context Protocol (MCP) server for native IDE/agent tool bindings.

## Core Dependencies & Storage
- **Database**: SQLite (`rusqlite`, `r2d2_sqlite`, `sqlitegraph`)
- **Async Runtime**: `tokio`
- **Serialization**: `serde`, `serde_json`, `bincode`
- **Networking & Transport**: `hyper`, `reqwest`, `tower`, `axum` (web UI / HTTP), `rmcp` (MCP protocol)
- **Hashing & Crypto**: `sha2`, `hex`

## Code Quality & Verification Gates
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Zero tolerance for `todo!`, `unimplemented!`, stubs, or dead-code suppressions.
