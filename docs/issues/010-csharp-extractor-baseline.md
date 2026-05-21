# Wire C# extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/csharp/` and the graph builder to emit C# symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-csharp.md`, `docs/references-csharp.md`, and `docs/attrs-csharp.md` for baseline coverage only.

XML doc comments (`/// <summary>...`) attach via `comment.documents_id` + `is_doc = true`.

End-to-end: parsing `benchmarks/csharp/dotnet-api/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] XML doc comments attach via `comment.documents_id` + `is_doc = true`
- [ ] `calls` rows match the new schema
- [ ] `using` directives produce both `raw_import` and resolved `imports` rows
- [ ] Snapshot test at `tests/snapshots/csharp/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
