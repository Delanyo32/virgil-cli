# Call-Resolution Tiers (Tier 1 + Tier 2) — Design

**Date:** 2026-05-30
**Branch:** `feat/call-resolution-tiers`
**Status:** Design approved, pending spec review

## Problem

virgil resolves calls by **name only, no type info** (documented heuristic in
CLAUDE.md). For a named-receiver call like `authService.hashPassword()` it
cannot tell what `authService` is, so it only links the call if a symbol of
that name is either (a) in the same file or (b) `exported` from a file the
caller's file `imports`. Named-receiver calls are 62–91% of all calls in every
language measured, so `call_edge` coverage is low and uneven.

Measured `edge_callees / functions` (fraction of functions reachable via
`call_edge`), cold build, 2026-05-30:

| Corpus | Lang | reachable | call_edge density (edges/fns) |
|--------|------|-----------|-------------------------------|
| spring-api | Java | 50% | 1.30 |
| http-service | Go | 47% | 0.65 |
| nextjs-dashboard | TS | 39% | 0.65 |
| technical-debt | Python | 34% | 1.63 |
| dotnet-api | C# | 25% | 0.55 |
| systems-cli | Rust | 26% | 0.36 |
| embedded-sensors | C | 21% | 0.39 |
| laravel-store | PHP | 17% | 0.32 |
| express-api | JS | 8% | 0.11 |
| data-processor | C++ | 6% | 0.08 |

No language exceeds 50%. The reachability/dead-code queries built on
`call_edge` therefore produce many false "unreachable" results — worst for JS
(8%) and C++ (6%).

The dense/sparse split is **not** a configured per-language setting. It is an
emergent property of the single name+import heuristic:
- A call resolves only via self-receiver (`this`/`self`), same-file name, or
  exported+imported name.
- JS is worst because both pillars of the cross-file path are weak: only
  53/213 functions get `exported=true` (CommonJS `module.exports` not
  captured), and only 77 import rows resolve for 55 files (`require()` paths).

(Note: CLAUDE.md and the skill docs currently claim `call_edge` is *dense for
JS/TS, sparse for Java/C#*. The data shows the inverse. That claim should be
corrected separately — out of scope for this prototype.)

## Goal

Raise `call_edge` coverage **without regressing cold-build cost**.

**Success criteria:**
1. **Coverage up:** `call_edge` count and `edge_callees/fns` increase on the
   target corpora, and a manual spot-check confirms new edges point to the
   *correct* target (not name-collision inflation).
2. **Cost gate:** total cold-build wall time **and** max RSS each stay within
   **5%** of master.
3. **No lost edges:** every edge master produces is still produced (both tiers
   fall back to today's name-match when the new path yields nothing).

## Approach

Two tiers, both living almost entirely in the **post-parse resolve pass**
(`resolve_and_emit_call_edges` in `src/db/from_code_graph.rs`), which runs
after the memory-dominant parse/absorb phase has freed its per-worker scratch.

### Central architectural bet

Per CLAUDE.md, peak RSS is set by the parse phase, not the resolve phase.
Therefore adding fact hashmaps to the resolve pass should **not raise peak
RSS**. The benchmark tests this hypothesis *first*; if peak RSS turns out to be
resolve-bound, the approach must be rethought before trusting any coverage
number.

### Tier 1 — name/import fix (helps JS, Python, PHP)

No type info required; repairs the existing name+import path.

- **Export flagging (parse-time, small):** the TS/JS extractor
  (`src/languages/typescript/queries.rs`, which handles both JS and TS — there
  is no separate `javascript/` module) marks functions assigned via
  `module.exports = { foo }` / `exports.foo = ...` as `exported`. This is the
  *only* change that touches the parse phase: a few extra query matches plus a
  bool set. Existing same-name "mark it exported" logic (queries.rs:462) is the
  insertion point.
- **Import resolution (resolve-time):** improve `require('./x')` and relative
  path resolution feeding the `imports` table, in `resolve_import_to_node`
  (`src/graph/builder.rs:1138`).

### Tier 2 — type-aware resolution (helps Java, C#, TS, C++, Go, Rust)

One **language-agnostic** resolve-pass step over uniform fact tables — not six
language implementations. It benefits any language where `parameter.type_id` /
`field_type` are populated, which the data confirms for the six typed
languages.

For a named-receiver call `x.m()`:
1. Resolve the receiver `x`'s type: look up `x` via `binding` → its declaring
   `parameter.type_id` or `field_type`, then `type.display_name` /
   `canonical_name`.
2. Find the class symbol for that type.
3. Find method `m` declared on that class; if absent, walk `extends` /
   `implements` to find inherited `m`.
4. **Fallback:** if any step fails (unknown type, local var with no annotation,
   generic), fall through to today's name-match. Never lose an edge.

New facts loaded into the resolve pass: `parameter(type_id)`, `field_type`,
`type`, `binding`. These add resolve-pass memory — the quantity under test
against the peak-RSS bet.

**Known coverage limit (intentional for the prototype):** receivers typed only
via local assignment (`const x = new AuthService()`) have no `type_id` fact, so
they fall back to name-match. Constructor-param and field receivers (the
dependency-injection pattern dominant in Java/C#) *are* covered, because those
carry `parameter.type_id` / `field_type`. Local-variable type inference is
deferred (it would be Tier 3).

## Components touched

| File | Change | Phase |
|------|--------|-------|
| `src/languages/typescript/queries.rs` | Tier 1 export flagging for `module.exports`/`exports.x` | parse |
| `src/graph/builder.rs` (`resolve_import_to_node`, ~1138) | Tier 1 `require()`/relative import resolution | resolve |
| `src/db/from_code_graph.rs` (`resolve_and_emit_call_edges`, ~156) | Tier 2 type-aware receiver resolution + fallback | resolve |

No schema change — all facts Tier 2 needs (`parameter.type_id`, `field_type`,
`type`, `binding`) already exist and are populated.

## Benchmark plan

Compare **master** vs **`feat/call-resolution-tiers`** on one harness (adapted
`examples/bench_matrix.sh`; `/usr/bin/time -l` for max RSS; median-of-3 wall).

Report parse-phase wall/RSS **separately** from total, since the bet is that
parse is untouched and only resolve grows.

### Cost measurement (the 5% gate)

- **Corpus:** openclaw subsets (`discord` ~522, `ui` ~461 TS/TSX files) —
  present locally, large enough that wall/RSS exceed noise, and TS exercises
  the Tier 2 path.
- **Gap:** this corpus is TS-only — it measures Tier 2's cost well but not
  Tier 1 (JS) or Tier 2-on-Java at scale. The user will provide a larger
  single-language repo (e.g. a big JS and/or Java codebase) when needed to
  close this gap.
- **Metrics per branch:** total cold-build wall, parse-phase wall, max RSS,
  resolve-phase delta.

### Coverage measurement (correctness)

- **Corpus:** all 10 registered language extracts (small, fast).
- **Metrics:** `call_edge` count and `edge_callees/fns` per corpus,
  master vs prototype.
- **Correctness spot-check (mandatory):** confirm specific new edges are
  *correct*, e.g. express-api `login` resolves to the route-handler target and
  `hashPassword` resolves to `authService.hashPassword`, not an unrelated
  same-named symbol. Inflated-but-wrong edges count as a regression, not a win.

### Gaps flagged (not hidden)

1. Large cost corpus is TS-only until a bigger JS/Java repo is supplied.
2. No ground-truth call graph exists, so coverage gain is validated by counts +
   spot-checks, not a precision/recall figure against truth.

## Out of scope

- Tier 3 (local-variable type inference / flow analysis for dynamic languages).
- Correcting the wrong "JS/TS is dense" claim in CLAUDE.md and the skill docs
  (tracked separately).
- Incremental refresh, schema changes, new fact tables.
