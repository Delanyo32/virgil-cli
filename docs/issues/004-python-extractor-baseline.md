# Wire Python extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/python/` and the graph builder to emit Python symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-python.md`, `docs/references-python.md`, and `docs/attrs-python.md` for baseline coverage only.

End-to-end: parsing `benchmarks/python/technical-debt/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Docstring attachment for function/class symbols populates `comment.documents_id` + `is_doc = true`
- [ ] Decorator handling preserves the existing `decorated_definition` dedup behavior (per CLAUDE.md note)
- [ ] `calls` and `imports` rows match the new schema
- [ ] Snapshot test at `tests/snapshots/python/baseline.cozoql` passes against the benchmark corpus
- [ ] `cargo test` passes

## Blocked by

- #1
