# Level 3 extraction depth for `type` and `references`

The new schema includes a `type` relation (seven `kind` variants + canonical resolution) and a `references` relation (read/write/type_use/import_use, scope-aware resolution). We're committing to **Level 3** for both: full kind decomposition with canonical-name resolution for types, and full lexical scope walking with shadowing for references. Each language gets a pre-step contract document at `docs/{types,references}-<lang>.md` (worked examples against `../virgil-skills/benchmarks/<lang>/`), then a per-language subagent implements the extractor. Nine languages run in parallel per phase. We picked Level 3 over the cheaper Level 1 (raw textual stub) or Level 2 (shape without resolution) because the schema doc lists canonical names and resolved kinds as load-bearing for downstream queries, and shipping the relation with `kind = "named"` everywhere or empty `read`/`write` rows would silently lie to query authors.

## Considered options

- **Level 1 (textual stub for types, import_use+type_use only for references)** — cheapest, but most schema fields stay `null` and queries that look at `kind` or `ref_kind` return misleading results.
- **Level 2 (shape-aware types, heuristic-resolved references)** — middle ground; rejected because the marginal cost from Level 2 → Level 3 is concentrated in language-specific work that's already on the critical path.

## Consequences

- `type` rows dedup per file (`type.id = hash(language, file_id, raw_text)`); `canonical_name` is filled but is not the dedup key — cross-file aggregation joins through `canonical_name`.
- Unresolved types (parse failure, external symbol we haven't indexed, type parameter `T`) get `canonical_name = null` and a per-file id; queries can filter them out.
- `references.referent_id` is nullable (`String?`) and lives in the value position; the key is `(referrer_id, site_file, site_start_byte, match_index)`. `match_index = 0` for the primary candidate; overload resolution emits additional rows at `match_index = 1, 2, ...`. Identifiers that can't be resolved get a single row with `referent_id = null`.
- `references` resolution is per-language with no shared resolver — every language module owns its scope rules.
- Compound assignments (`x += 1`) emit a single `write` row, not `write + read`. This is a deliberate Level-3 narrowing; faithful read+write semantics would be Level 4.
- Field-level `read`/`write` rows are emitted only when the field has a known `symbol_id` in the store. Anonymous or non-extracted fields produce no `references` rows (compare to method calls, which always emit `calls` rows via name-based resolution).
- Pointer / reference types (`*T`, `&T`, `T*`, `T&`) across Rust/C/C++/Go all map to `kind = "generic"` with a single type argument — keeps the schema's 7-kind closed set intact.
- Failure mode for a language: snapshot tests for that language fail; the orchestrator re-dispatches the subagent against the same contract doc.
