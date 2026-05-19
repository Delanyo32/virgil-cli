# 09 — Drop petgraph from Cargo.toml

**Type:** AFK
**Label:** ready-for-agent

## Parent

`.planning/proposals/cozodb-migration.md` — Phase 9.

## What to build

After Phase 6 deleted the last consumers of `petgraph::DiGraph`, remove the
dependency.

Scope:

- Delete `petgraph` from `Cargo.toml`.
- Remove any lingering `use petgraph::...` imports the deletion PR missed.
- Confirm via `cargo tree` and `cargo build`.

## Acceptance criteria

- [ ] `petgraph` not in `Cargo.toml`
- [ ] `cargo tree | grep petgraph` returns nothing
- [ ] `cargo build` succeeds
- [ ] `cargo test` green

## Blocked by

- 06-the-deletion
