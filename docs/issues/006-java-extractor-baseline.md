# Wire Java extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/java/` and the graph builder to emit Java symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-java.md`, `docs/references-java.md`, and `docs/attrs-java.md` for baseline coverage only.

End-to-end: parsing `benchmarks/java/spring-api/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Javadoc comments attach via `comment.documents_id` + `is_doc = true`
- [ ] `calls` and `imports` rows match the new schema
- [ ] Snapshot test at `tests/snapshots/java/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
