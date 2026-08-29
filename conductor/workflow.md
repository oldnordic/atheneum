# Development Workflow: Atheneum

## Spec-Driven Development (SDD) Protocol
1. **Spec First**: Every non-trivial change or feature must be specified in `spec.md` with explicit functional requirements and acceptance criteria before implementation.
2. **Plan & Task Decomposition**: `plan.md` breaks work into hierarchical phases and tasks with fail-first test specifications.
3. **Fail-First TDD**:
   - Write tests asserting the required behavior.
   - Run tests and observe the expected failure before writing implementation code.
4. **Implementation & Refactor**:
   - Write minimal, modular, and idiomatic Rust code.
   - Avoid monolithic files; decompose large CLI modules into structured submodules.
5. **Phase Checkpoint & Verification**:
   - Run `cargo fmt --all -- --check`
   - Run `cargo clippy --workspace --all-targets -- -D warnings`
   - Run `cargo test --workspace`
   - Verify zero unhandled errors or stale documentation.
