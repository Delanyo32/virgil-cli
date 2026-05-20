# Wire Go extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/go/` and the graph builder to emit Go symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-go.md`, `docs/references-go.md`, and `docs/attrs-go.md` for baseline coverage only.

Go imports resolve to packages (not individual files) per the existing `GraphNode::Package` path. Comment-doc attachment follows the Go convention (comment directly above the declaration, no blank line).

End-to-end: parsing `benchmarks/go/http-service/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Go doc comments correctly attached via `comment.documents_id`
- [ ] `calls` and `imports` rows match the new schema (imports keyed to package path)
- [ ] Snapshot test at `tests/snapshots/go/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
