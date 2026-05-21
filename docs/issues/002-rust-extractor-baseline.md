# Wire Rust extractor into new schema (baseline)

**Label:** enhancement
**Type:** AFK

## What to build

Update `src/languages/rust_lang/` and the graph builder to emit Rust symbols, spans, comments, calls, and imports against the new schema (issue #1). Honor the contract in `docs/types-rust.md`, `docs/references-rust.md`, and `docs/attrs-rust.md` for baseline coverage only — type expressions, attrs, class hierarchy, and references resolution are out of scope here (they land in later issues).

The slice is end-to-end: parsing the `benchmarks/rust/systems-cli/` corpus produces the exact rows committed in the snapshot below.

## Acceptance criteria

- [ ] Rust `symbol` rows use String IDs in the ADR-0002 format (`path|start_line|start_col|name|kind`)
- [ ] Rust `span` rows include `start_byte`, `end_byte`, `start_line`, `end_line`, `start_col`, `end_col`
- [ ] Rust `comment` rows populate `documents_id` (when the comment attaches to a symbol), `is_doc`, `todo_kind`
- [ ] Rust `calls` rows include `call_site_file`, `call_site_start_byte`, `call_site_end_byte`, `is_direct`
- [ ] Rust `imports` rows match the new `(importer_file_id, imported_id)` shape; raw imports go to `raw_import` as today
- [ ] Snapshot test at `tests/snapshots/rust/baseline.cozoql` produces committed expected rows when run against `../virgil-skills/benchmarks/rust/systems-cli/`
- [ ] `cargo test` passes

## Blocked by

- #1
