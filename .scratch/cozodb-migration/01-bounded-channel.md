# 01 — Bounded channel between parse workers and absorber

**Type:** AFK
**Label:** ready-for-agent

## Parent

`.planning/proposals/cozodb-migration.md` — Phase 1.

## What to build

Switch the parse-worker → graph-absorber channel in `GraphBuilder::build` from
an unbounded `mpsc::channel` to a bounded `mpsc::sync_channel(N)` so peak
memory while building large workspaces is capped by backpressure, not by how
fast workers produce `FileGraphData`.

This is independent of the Cozo migration — it's a memory win that stands on
its own, and the same channel is what later phases will keep feeding (just
with a Cozo writer on the other end).

Choose `N` based on `num_cpus` (proposal suggests `2 * num_cpus`); document
the choice in the build module.

## Acceptance criteria

- [ ] Parse-worker channel uses `mpsc::sync_channel` with a documented bound
- [ ] `cargo test` green
- [ ] Build of a representative large workspace (e.g. openclaw) completes
      successfully and shows lower peak RSS than current `master`
- [ ] No behavioral change to the resulting `CodeGraph` (same nodes/edges)

## Blocked by

None — can start immediately.
