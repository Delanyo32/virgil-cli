# Wire PHP extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/php/` and the graph builder to emit PHP symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor `docs/types-php.md`, `docs/references-php.md`, and `docs/attrs-php.md` for baseline coverage only.

PHP grammar is `LANGUAGE_PHP` (handles `<?php` tags) per CLAUDE.md. PHPDoc comments use `/** ... */` blocks and attach to the next declaration.

End-to-end: parsing `benchmarks/php/laravel-store/` produces the rows committed in the snapshot.

## Acceptance criteria

- [ ] Symbol rows use ADR-0002 String IDs; spans include byte + column offsets
- [ ] PHPDoc comments attach via `comment.documents_id` + `is_doc = true`
- [ ] `calls` and `imports` rows match the new schema
- [ ] Snapshot test at `tests/snapshots/php/baseline.cozoql` passes
- [ ] `cargo test` passes

## Blocked by

- #1
