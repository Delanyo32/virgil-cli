# Wire C++ extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/cpp/` and the graph builder to emit C++ symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-cpp.md`, `docs/references-cpp.md`, and `docs/attrs-cpp.md` for baseline coverage only.

C++ headers use `.hpp`/`.hxx`/`.hh` (`.h` maps to C). Doc comments use Doxygen-style `/** ... */` or `///`.

End-to-end: parsing `benchmarks/cpp/data-processor/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Doxygen comments attach via `comment.documents_id` + `is_doc = true`
- [ ] `calls` rows match the new schema
- [ ] `#include` directives produce both `raw_import` and resolved `imports` rows
- [ ] Snapshot test at `tests/snapshots/cpp/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
