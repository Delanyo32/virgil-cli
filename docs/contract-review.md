# Contract docs — cross-language review

After 9 subagents wrote `docs/{types,references,attrs}-<lang>.md` independently, this review consolidates the cross-cutting decisions they made and the inconsistencies that need reconciliation before Phase 1 begins. Each item flags whether it's a **schema bug** (the schema doc itself needs updating), a **policy choice** (subagents diverged, pick one), or a **per-language gap** (one language needs follow-up work in isolation).

---

## Critical: schema bugs

### 1. `references.referent_id` nullability — **schema bug**

The schema doc declares:

```
:create references {
    referrer_id: String,
    referent_id: String =>
    ...
}
```

`referent_id` is part of the relation key and non-nullable. But Python, TypeScript, PHP, and C# contracts all rely on `null` to mark unresolvable referents (Eloquent magic properties, untyped locals, closure-captured non-symbol variables, CommonJS destructured-`require` bindings). C++ flagged this directly.

Java picked "skip the row" — the only language that doesn't need nullability.

**Reconcile.** Three options:

- **A. Update schema:** move `referent_id` out of the key into the value position and declare it `String?`. Cross-relation keys change shape.
- **B. Sentinel string:** standardize on `"<unresolved>"` for unresolvable referents. Keeps the key shape but loses the ability to distinguish "we tried and failed" from "we never tried."
- **C. Standardize on skip:** all extractors drop rows with unresolvable referents, matching Java. Loses signal that a reference *exists* but is unbound.

**Recommendation: A, with revision.** Moving `referent_id` to value position alone breaks overload resolution — C++ contracts emit multiple rows at the same site, distinguished only by `referent_id`. Real design adds `match_index: Int` to the key: `match_index = 0` for the primary candidate, `1+` for additional overloads, and `referent_id` becomes `String?` in the value position.

**Landed:** schema doc updated; ADR-0003 consequences section updated.

### 2. PHP property→type linkage — **schema gap**

PHP supports typed properties (`private string $name;`). The current schema has `parameter`, `returns_type`, `throws` linking symbols to types — but no relation linking a property/field symbol to its declared type. Same gap exists for TypeScript class fields, Java fields, C# fields, Rust struct fields, Go struct fields, C/C++ struct members.

**Reconcile.** Add a `field_type {symbol_id => type_id}` relation to the schema. PHP, Rust, C++, Java, C#, TS, Go all need it. Defer to Phase 2 of the schema work (with the symbol-metadata expansion) — not Phase 1.

**Landed:** `field_type` relation added to schema doc.

---

## Policy choices to standardize

### 3. Pointer / reference encoding — **policy choice**

Subagents diverged 2-2 on how pointer types map to schema `kind` variants:

| Language | Choice |
|---|---|
| Rust | `&T`, `&mut T`, `*const T`, `*mut T` → `kind = generic` (one arg = referent) |
| C | `T*` → `kind = generic`, synthetic `ptr<T>` wrapper |
| C++ | `T*`, `T&` → `kind = named`, literal `*`/`&` in display_name |
| Go | `*T` → kind unspecified, `*` retained in display_name |

**Reconcile.** Pick one of:

- **A. `generic` everywhere** (Rust/C path): pointers are a generic over one type argument. C++ and Go contracts updated. Pro: uniform; queries can filter `kind = "generic"` and get all pointer-ish types across languages. Con: stretches the `generic` definition past template/parametric types.
- **B. `named` everywhere** (C++ path): pointer types are opaque named types whose `display_name` contains the punctuation. Pro: simple, matches what tree-sitter naturally produces. Con: lose the structural info that the type is over a referent.

**Recommendation: A.** The schema doc's 7 kinds are a closed set; without `generic` for pointers we'd need to add a `pointer` kind or admit that pointer types are unstructured. A keeps the closed set and exposes structure. Update C++ and Go contracts to use `generic`.

### 4. Compound assignment `ref_kind` — **policy choice**

`x += 1` semantically reads `x` then writes `x`. Subagents picked:

- Rust: single `write` row
- TS: single `write` row (first-match-wins)
- Others: didn't address

**Reconcile.** Two options:

- **A. Single `write` row** (Rust/TS path): treat compound assignment as a write only. Queries looking for reads miss `x += 1`.
- **B. Two rows: one `read`, one `write`**: faithful to semantics. Doubles the row count for a common pattern.

**Recommendation: A.** Faithful semantics is a Level 4 commitment. Level 3 picks the dominant kind. Document it explicitly in every references doc; update the 7 docs that didn't say so.

### 5. `*_attrs` columns vs `symbol` columns — **policy choice**

The `symbol` relation already has `is_async`, `is_static`, `is_abstract`, `is_mutable`. Some subagents added overlapping columns to their `*_attrs` tables:

- PHP: explicitly avoided duplicating `is_abstract` / `is_static`
- Java: duplicated `throws_clause` (raw textual) alongside the resolved `throws` relation
- C: added `is_static`, `is_extern`, `is_inline` — `is_static` overlaps with `symbol.is_static`

**Reconcile.** Policy: **no `*_attrs` column duplicates a `symbol` column.** When a language needs a more-specific variant, give it a different name (e.g. `c_attrs.is_file_static` for C's file-scoped statics, distinct from `symbol.is_static`).

The Java `throws_clause` case is the exception worth keeping: the raw textual list is genuinely different data from the resolved `throws` rows (the textual list is the source of truth when the exception type isn't indexed).

### 6. Field-level read/write tracking — **policy choice**

The Level 3 references commitment says "scope-aware resolution per language." But what counts as a `read` / `write` when the target is a *field* rather than a local?

| Language | Choice |
|---|---|
| Go | Selector RHS (`x.Field`) emits no row; assignment `x.Field = v` writes against the channel/pointer name only |
| Rust | Mutating receivers detected via stdlib mutator-name whitelist |
| C# | Auto-property reads/writes recorded against the property *symbol* (no synthetic accessors) |
| Others | Various |

**Reconcile.** Single policy: **field access produces a `read`/`write` row when the field has a known `symbol_id` in the store; otherwise no row.** This matches C# de facto. Update Go (record field rows when the struct field is in `symbol`), Rust (drop the mutator whitelist heuristic; only structural access counts), and the others.

This narrows Level 3 — it admits that fields not extracted as symbols (most local-struct fields) won't have references rows. That's honest and aligned with the Phase 2 commitment to extract field symbols (see Phase 2 plan).

---

## Per-language follow-up gaps

### 7. Java: extractor doesn't emit parameter/local symbols

The Java subagent flagged that `src/languages/java/queries.rs` doesn't currently extract parameter or local-variable definitions. Without those `symbol` rows, the references resolver can't bind identifiers to anything inside a method.

**Action:** Phase 2 of Java implementation extends the queries to emit parameter + local symbols before the references walker can produce meaningful rows. Other languages may have the same gap — audit each `queries.rs` before Phase 2 dispatch.

### 8. C23 `[[attribute]]` syntax punted to `symbol_attr`

C subagent committed `__attribute__((...))` capture to `c_attrs.gcc_attributes` but punted C23 bracketed attributes to the generic `symbol_attr` escape hatch until tree-sitter-c surfaces a distinct node kind.

**Action:** acceptable as written. Revisit when tree-sitter-c upgrades.

### 9. Synthesized worked examples

Python, TypeScript, C, and C++ contracts include worked examples that are *clearly labeled* as synthesized rather than drawn from the benchmark corpus. Caused by the corpora not exercising every schema `kind` or `ref_kind`.

**Action:** acceptable for Phase 0. Each phase's snapshot tests can use synthesized fixtures committed to `tests/snapshots/<lang>/` alongside the real benchmark snapshots. Don't grow the benchmark corpora for this.

---

## Decisions confirmed consistent across all 9

Worth recording so future contributors don't relitigate:

- **Type alias canonicalization:** aliases stay as themselves; `type Foo = Vec<u8>` makes `Foo` canonicalize to `Foo`, not the expansion. (Rust, TS, Python all explicit; others compatible.)
- **`*_attrs` separation of concerns:** language-specific flags live in `*_attrs`; cross-language flags live on `symbol`. (All 9.)
- **Anonymous symbols attribute references to the enclosing named symbol** as `referrer_id`. (TS explicit; others compatible with ADR-0002's name requirement.)
- **Overload resolution emits one row per candidate**, sharing `referrer_id` + site. (C++ explicit, C# explicit, others compatible.)
- **`auto`, `decltype`, untyped parameters → `canonical_name = null`** rather than synthesizing types. (C++, Python, TS all explicit.)

---

## Update — Datalog resolution pivot ([ADR-0005](adr/0005-datalog-resolution.md))

After this review landed, a follow-up architectural decision moved **all symbol resolution out of per-language Rust extractors** and into Cozoscript rules over a new fact-emission factbase (`occurrence` / `scope` / `binding`). The 9 `references-<lang>.md` contracts get rewritten to describe **what facts to emit**, not **how to resolve names**. The Level-3 commitment from ADR-0003 is preserved — what changes is where the algorithm lives.

Per-language items 3–6 from this review (pointer encoding, compound assignment policy, field-row policy, attrs-vs-symbol overlap) remain valid — they govern fact emission and types, which ADR-0005 does not affect.

## Outcome

Once items 1–6 are resolved (schema-doc edits + 7-language contract amendments), Phase 0 documentation is locked. Phases 1–6 of the implementation plan can dispatch against these contracts without further policy churn.

**Next actions (sequence matters):**
1. Update `docs/virgil-datalog-schema.md`: change `references.referent_id` to value-position `String?`, add `field_type` relation.
2. Update [ADR-0003](adr/0003-level-3-types-and-references.md) consequences section to reflect nullable `referent_id`.
3. Amend the 8 language contracts that need touching for items 3–6 (probably a single agent pass, not 9 parallel).
4. Audit each `src/languages/<lang>/queries.rs` for the "parameter/local symbol missing" gap surfaced by Java (item 7).
