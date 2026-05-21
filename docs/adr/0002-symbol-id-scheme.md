# Symbol ID scheme: `path|start_line|start_col|name|kind`

Symbol IDs in the new schema are `String`. We construct them as `path|start_line|start_col|name|kind` (pipe-joined). All inputs come straight from tree-sitter, so no dependency on later phases. The id is human-readable in query output, stable across edits below the symbol, and stable enough for incremental refresh to skip unchanged symbols. `start_col` is included from Phase 1 onward to eliminate realistic collisions (anonymous functions chained on a single line, macro-expanded symbols reporting the same line).

## Considered options

- **`path|kind|qualified_name`** — most stable across edits but blocks on per-language qualified-name extraction landing first.
- **Opaque hash** (`blake3(...)`) — compact and uniform, but unreadable in query output and offers no debugging affordance.
- **Sequential `s:<n>`** — cheapest migration, but no stability across rebuilds and no incremental-refresh benefit.

## Consequences

- Edits *above* a symbol shift its `start_line`/`start_col` and therefore its id. Incremental refresh re-emits these rows — acceptable today since the refresh path already re-resolves cross-file edges from facts.
- `file.id` is the path itself (no synthetic file id); avoids an extra join layer.
- A future move to `qualified_name`-based ids is additive: only the id formatter changes; the writer interface is unaffected.
