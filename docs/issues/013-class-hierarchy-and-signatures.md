# Class hierarchy + signatures: extends, implements, parameter.type_id, returns_type, throws, Level-3 `type` relation (parity)

**Label:** enhancement
**Type:** AFK

## What to build

Populate the relations that describe class hierarchies and function signatures end-to-end:

- `extends` — child-to-parent class relationships (single inheritance languages) or trait/interface inheritance (Rust traits, TS interfaces)
- `implements` — class-to-interface relationships
- `parameter.type_id` — link function parameters to their declared types
- `returns_type` — function return types
- `throws` — declared exceptions (Java, C#, PHP @throws)
- `type` relation — populated to Level 3 (full kind decomposition + canonical resolution) per ADR-0003

Per ADR-0003, all 9 languages land together. Per-language type-expression mapping rules live in `docs/types-<lang>.md`. Canonical-name resolution depends on the `imports` relation populated by issues #2–#10.

Dispatch one subagent per language with the contract doc + benchmark corpus.

## Acceptance criteria

- [ ] `extends` rows populated for class/interface/trait inheritance in all 9 languages
- [ ] `implements` rows populated for class-implements-interface relationships where the language has them
- [ ] `parameter.type_id` populated whenever the language has a type annotation; `null` for untyped parameters (Python, JS, dynamic PHP)
- [ ] `returns_type` rows populated for annotated functions; absent for unannotated
- [ ] `throws` rows populated for languages with declared exceptions (Java, C#, PHP)
- [ ] `type` rows include `kind` (one of the 7 schema variants), `display_name`, and resolved `canonical_name` (`null` when unresolvable per ADR-0003)
- [ ] Pointer/reference types (`*T`, `&T`, `T*`, `T&`) emitted as `kind = "generic"` with one argument per contract review policy 2
- [ ] Per-language snapshot tests at `tests/snapshots/<lang>/types-and-hierarchy.cozoql` validate expected rows
- [ ] `cargo test` passes

## Blocked by

- #12
