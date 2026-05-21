# Wire C extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/c_lang/` and the graph builder to emit C symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-c.md`, `docs/references-c.md`, and `docs/attrs-c.md` for baseline coverage only.

`.h` files map to C (not C++) per CLAUDE.md. `#include` directives flow through `imports` / `raw_import`.

End-to-end: parsing `benchmarks/c/embedded-sensors/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Doc comments (`/** ... */`) attach via `comment.documents_id`
- [ ] `calls` rows match the new schema
- [ ] `#include` directives produce both `raw_import` and resolved `imports` rows
- [ ] Snapshot test at `tests/snapshots/c/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
