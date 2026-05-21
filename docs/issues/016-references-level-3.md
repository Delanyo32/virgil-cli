# References (Level 3): read/write/type_use/import_use with scope-aware resolution (parity)

**Label:** enhancement
**Type:** AFK

## What to build

Populate the `references` relation at Level 3 per ADR-0003: every identifier occurrence in source code emits a row, with `ref_kind` ∈ {`read`, `write`, `type_use`, `import_use`} and `referent_id` resolved via per-language lexical scope walking.

Per-language scope rules and `ref_kind` decision trees live in `docs/references-<lang>.md`. Resolution uses the parameter/local symbols extracted in issue #11 and the type-use rows seeded by issue #13.

Policy specifics (per contract review):
- `referent_id` is nullable in the value position; key is `(referrer_id, site_file, site_start_byte, match_index)`.
- Overload candidates emit additional rows at `match_index = 1, 2, ...`.
- Compound assignments (`x += 1`) emit a single `write` row, not `write + read`.
- Field-level `read`/`write` rows emitted only when the field has a known `symbol_id`.

Dispatch one subagent per language with the contract doc + benchmark corpus.

## Acceptance criteria

- [ ] All four `ref_kind` values populated for every language (where the language supports the concept)
- [ ] Scope-aware resolution: shadowing handled per `docs/references-<lang>.md`
- [ ] Unresolvable identifiers produce a row with `referent_id = null` (no language uses "skip" any more)
- [ ] Overload candidates use `match_index` to disambiguate
- [ ] Compound assignments emit single `write` row across all 9 languages
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/references.cozoql` validate expected rows (including shadowing and non-local writes)
- [ ] `cargo test` passes

## Blocked by

- #11, #13
