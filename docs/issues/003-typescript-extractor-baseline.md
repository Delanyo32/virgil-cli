# Wire TypeScript extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/typescript/` and the graph builder to emit TypeScript/JavaScript symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor the contracts in `docs/types-typescript.md`, `docs/references-typescript.md`, and `docs/attrs-typescript.md` for baseline coverage only. JS files emit zero `type` rows; both `.ts/.tsx` and `.js/.jsx` flow through this extractor.

End-to-end: parsing `benchmarks/typescript/nextjs-dashboard/` and `benchmarks/javascript/express-api/` produces the rows committed in the snapshots.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] Comments populate `documents_id`, `is_doc` (JSDoc detection), `todo_kind`
- [ ] `calls` rows include byte offsets and `is_direct`
- [ ] `imports` rows match the new shape; CJS destructured-require behavior as specified in `references-typescript.md`
- [ ] Snapshot tests at `tests/snapshots/typescript/baseline.cozoql` and `tests/snapshots/javascript/baseline.cozoql` pass against the corresponding benchmark corpora
- [ ] `cargo test` passes

## Blocked by

- #1
