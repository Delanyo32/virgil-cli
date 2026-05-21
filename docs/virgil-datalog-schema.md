# Virgil-CLI Datalog Schema

The Datalog/CozoScript equivalent of the RDF ontology for the virgil-cli code analysis layer, covering Rust, Python, PHP, C, C#, C++, Go, TypeScript, and JavaScript.

---

## The core modeling shift

In RDF, every attribute is a separate triple sharing a subject:

```turtle
v:fn:parse_tree
    a code:Function ;
    code:name "parse_tree" ;
    code:language "rust" ;
    code:visibility "public" ;
    code:isAsync false ;
    code:definedIn v:file/parser.rs .
```

In Datalog, the same information is **one row** in a typed relation:

```
:create function {
    id: String          =>
    name: String,
    language: String,
    visibility: String,
    is_async: Bool,
    file_id: String,
}
```

The `=>` separates primary-key columns from value columns in CozoScript.

## What collapses from RDF to Datalog

| RDF concept | Datalog equivalent |
|---|---|
| Class (`code:Function`) | A relation, OR a `kind` column on a generic `symbol` relation |
| Subclass (`Method rdfs:subClassOf Callable`) | Either a discriminator column with values like `"function"`, `"method"` — or two relations with similar columns |
| Property (`code:calls`) | A relation with `(from, to)` columns |
| Transitive property (`code:calls+`) | A recursive rule defined at query time, not declared in the schema |
| Object property | A column referencing another relation's primary key |
| Datatype property | A scalar-typed column |
| Inverse property (`owl:inverseOf`) | Either flip the columns in a query, or store both directions explicitly |

The big shift: in RDF the schema declares relationships *and* their transitive/inverse properties. In Datalog, schema declares *shape only*; relationships like transitivity are expressed in queries via recursive rules.

---

## Core schema

```
# ===== SYMBOLS =====
# One row per named entity in the codebase.

:create symbol {
    id: String =>
    kind: String,              # "function", "method", "class", "interface", "enum",
                               # "struct", "type_alias", "variable", "parameter",
                               # "field", "constant", "constructor", "closure"
    name: String,
    qualified_name: String,
    language: String,
    visibility: String,        # "public", "private", "internal", "protected"
    file_id: String,
    parent_id: String?,        # containing symbol (class for a method, module for a function)
    is_async: Bool default false,
    is_static: Bool default false,
    is_abstract: Bool default false,
    is_mutable: Bool default false,
}

# ===== FILES & SPANS =====

:create file {
    id: String =>
    path: String,
    language: String,
    repo_id: String,
}

:create span {
    entity_id: String,         # the symbol/comment/etc this span belongs to
    file_id: String =>
    start_byte: Int,
    end_byte: Int,
    start_line: Int,
    end_line: Int,
    start_col: Int,
    end_col: Int,
}

# ===== EDGES (the graph part) =====
# Each edge type gets its own relation, not a generic `edge` table.

:create calls {
    caller_id: String,
    callee_id: String =>
    call_site_file: String,
    call_site_start_byte: Int,
    call_site_end_byte: Int,
    is_direct: Bool,
}

# `references` is a DERIVED view per ADR-0005, computed by Cozoscript
# rules in `docs/resolution.md` from `occurrence` + `scope` + `binding`
# + `imports`. Extractors do NOT populate this relation directly.
# The shape is preserved so downstream queries against `*references{...}`
# keep working; the resolver materialises the rows on (re)build.
:create references {
    referrer_id: String,
    site_file: String,
    site_start_byte: Int,
    match_index: Int =>         # 0 for the only/primary candidate;
                                # 1+ for additional overload candidates at the same site
    referent_id: String?,       # null when the identifier can't be resolved to a symbol
    ref_kind: String,           # "read", "write", "type_use", "import_use"
}

# ─── ADR-0005 fact-emission relations ──────────────────────────────────
# Populated by per-language extractors. The Cozoscript resolver consumes
# these to materialise `references` (and, in a later phase, `calls`).

# Every identifier occurrence in source code. Resolution turns each
# occurrence into zero-or-more `references` rows via scope + import
# walking.
:create occurrence {
    id: String =>              # `path|start_byte|name|occurrence_kind`
    name: String,              # textual identifier as written in source
    file_path: String,
    start_byte: Int,
    end_byte: Int,
    enclosing_symbol_id: String?,  # innermost symbol containing the occurrence
    enclosing_scope_id: String,    # innermost lexical scope
    occurrence_kind: String,   # "call", "read", "write", "type_use", "import_use"
}

# Lexical scope chain per file. `parent_id = null` for the file/module
# scope. Each scope has a kind so the resolver can apply per-kind rules
# (e.g. function parameters shadow module bindings).
:create scope {
    id: String =>              # `file_path|start_byte|kind`
    parent_id: String?,
    file_path: String,
    kind: String,              # "file", "module", "namespace", "class",
                               # "function", "block"
    start_byte: Int,
    end_byte: Int,
}

# A name → symbol_id binding within a specific scope. Covers
# definitions (`fn foo` binds `foo` in its enclosing scope), parameter
# declarations, import aliases (`import { foo as bar }` binds `bar`),
# and wildcard imports (one row per imported file with name = "*").
#
# Multiple bindings to the same (scope_id, name) are allowed when the
# language permits shadowing in the same scope (Rust `let` rebinding);
# the resolver picks by `start_byte` order.
:create binding {
    scope_id: String,
    name: String,
    start_byte: Int =>         # disambiguator + ordering key
    symbol_id: String?,        # null when the target is external/unknown
    binding_kind: String,      # "definition", "parameter", "import",
                               # "import_alias", "wildcard_import"
}

:create extends {
    child_id: String,
    parent_id: String,
}

:create implements {
    impl_id: String,
    interface_id: String,
}

:create imports {
    importer_file_id: String,
    imported_id: String,        # could be a module, package, or file
}

# ===== SIGNATURES =====

:create parameter {
    function_id: String,
    index: Int =>
    name: String,
    type_id: String?,
    is_optional: Bool,
    has_default: Bool,
}

:create returns_type {
    function_id: String =>
    type_id: String,
}

:create throws {
    function_id: String,
    exception_type_id: String,
}

# field_type links a field/property/struct-member symbol to its declared type.
# Distinct from `parameter` (which is for function parameters) and `returns_type`
# (function returns). Covers: Rust struct fields, Go struct fields, Java/C# fields,
# TypeScript class fields, PHP typed properties, C/C++ struct members.
# Untyped fields (e.g. JS, dynamic PHP) emit no row.
:create field_type {
    symbol_id: String =>
    type_id: String,
}

# ===== TYPES =====

:create type {
    id: String =>
    kind: String,               # "primitive", "named", "generic", "union",
                                # "intersection", "function", "tuple", "array"
    language: String,
    display_name: String,
    canonical_name: String?,    # for named types, the fully-qualified name
}

# ===== COMMENTS =====

:create comment {
    id: String =>
    documents_id: String?,      # the symbol this comment is attached to, if any
    file_id: String,
    kind: String,               # "line", "block", "doc"
    is_doc: Bool,
    text: String,
    todo_kind: String?,         # "TODO", "FIXME", "XXX", "HACK"; null if not a TODO
    start_byte: Int,
    end_byte: Int,
}
```

---

## Language-specific extensions

In RDF you had `rust:isUnsafe`, `py:docstringStyle`, `cpp:isVirtual`. In Datalog there are two patterns; use both:

### Pattern 1 — sparse extension tables (hot attributes)

One relation per language, keyed by symbol id. Rows only exist for symbols of that language. Use this for attributes you query often.

```
:create rust_attrs {
    symbol_id: String =>
    is_unsafe: Bool default false,
    is_const: Bool default false,
    derives: [String] default [],     # list of trait names
}

:create python_attrs {
    symbol_id: String =>
    decorators: [String] default [],
    is_generator: Bool default false,
    is_coroutine: Bool default false,
    docstring_style: String?,         # "google", "numpy", "sphinx"
}

:create typescript_attrs {
    symbol_id: String =>
    is_readonly: Bool default false,
    is_optional: Bool default false,
    type_parameters: [String] default [],
}

:create cpp_attrs {
    symbol_id: String =>
    is_virtual: Bool default false,
    is_const: Bool default false,
    is_noexcept: Bool default false,
    is_template: Bool default false,
}

:create csharp_attrs {
    symbol_id: String =>
    attributes: [String] default [],   # C# attributes (annotations)
    is_partial: Bool default false,
    is_sealed: Bool default false,
}

:create go_attrs {
    symbol_id: String =>
    is_exported: Bool default false,   # capitalized name
    has_receiver: Bool default false,
    build_tags: [String] default [],
}

:create php_attrs {
    symbol_id: String =>
    is_final: Bool default false,
    uses_traits: [String] default [],
}

:create c_attrs {
    symbol_id: String =>
    is_static: Bool default false,
    is_extern: Bool default false,
    is_inline: Bool default false,
}
```

### Pattern 2 — generic key-value extension (long tail)

One relation that holds arbitrary `(symbol_id, key, value)` rows. Use this as an escape hatch for rare, infrequently-queried attributes.

```
:create symbol_attr {
    symbol_id: String,
    key: String =>
    value: String,
}
```

The rule of thumb: typed columns where it matters, flexibility where it doesn't. This is exactly the kind of choice Datalog lets you make and RDF doesn't — RDF requires triple-ifying everything regardless of cardinality.

---

## Query translations

Queries from the SPARQL version, rewritten in CozoScript.

### Public functions missing documentation

```
?[fn_id, fn_name] :=
    *symbol{id: fn_id, kind: 'function', visibility: 'public', name: fn_name},
    not has_doc[fn_id]

has_doc[fn_id] := *comment{documents_id: fn_id, is_doc: true}
```

The idiom: define the negative-target predicate as its own rule, then negate it. Cozo uses stratified negation here.

### Transitive callers of `unsafe_block`

```
reachable[caller] := *calls{caller_id: caller, callee_id: 'unsafe_block'}
reachable[caller] := reachable[intermediate], *calls{caller_id: caller, callee_id: intermediate}

?[caller] := reachable[caller]
```

This is the place SPARQL has the syntactic edge — `:calls+` versus the explicit recursive rule. But the Datalog version is more *honest* about what's happening: it's a transitive closure, and you control how it's computed (e.g., bounded depth by adding `depth < 10` to the recursive case).

### All TODOs grouped by kind

```
?[kind, count(c_id)] :=
    *comment{id: c_id, todo_kind: kind},
    kind != null
```

### Rust unsafe functions whose doc comments don't mention "SAFETY"

```
?[fn_id] :=
    *symbol{id: fn_id, kind: 'function', language: 'rust'},
    *rust_attrs{symbol_id: fn_id, is_unsafe: true},
    not has_safety_note[fn_id]

has_safety_note[fn_id] :=
    *comment{documents_id: fn_id, text: t},
    str_includes(t, 'SAFETY')
```

`str_includes` is CozoScript's substring builtin. The "define positive rule, then negate it" pattern generalizes — it's how Datalog cleanly expresses "not exists" without SPARQL's explicit `FILTER NOT EXISTS`.

### Cross-module heavy callees (compositional query)

```
cross_module_call[caller, callee] :=
    *calls{caller_id: caller, callee_id: callee},
    *symbol{id: caller, file_id: f1},
    *symbol{id: callee, file_id: f2},
    *file{id: f1, path: p1},
    *file{id: f2, path: p2},
    module_of(p1) != module_of(p2)

heavy_callable[fn] :=
    *parameter{function_id: fn, index: i},
    i >= 3

?[caller, callee] :=
    cross_module_call[caller, callee],
    heavy_callable[callee]
```

This is where Datalog's compositional rules shine. You name `cross_module_call` and `heavy_callable` as concepts, then intersect them in the final query. SPARQL would jam this into one giant pattern.

### Call depth from each function to its leaves (recursive aggregation)

```
depth[fn, 0] :=
    *symbol{id: fn, kind: 'function'},
    not *calls{caller_id: fn, callee_id: _}

depth[fn, max(d + 1)] :=
    *calls{caller_id: fn, callee_id: callee},
    depth[callee, d]

?[fn, d] := depth[fn, d]
```

This is Datalog's killer feature for code analysis: **recursive aggregation**. SPARQL property paths handle "is there a path?" well, but cannot natively aggregate values *along* a recursive computation. Cozo's stratified semantics make this safe and well-defined.

---

## RDF vs Datalog: side-by-side

| Aspect | RDF/SPARQL | Datalog/Cozo |
|---|---|---|
| Add a new attribute to all symbols | Add a property; no migration | Add a column with default; migrate the relation |
| Add a new language extension | New namespace, new properties | New extension relation |
| Add a new edge type | New property | New relation |
| Multi-column edges (e.g. call-site location) | Reify the edge as a node, attach properties | Just add columns to the edge relation |
| Subclass reasoning (is X a `Callable`?) | Built-in via subclass inference | Explicit: `kind IN ('function', 'method', 'constructor')` |
| Transitive reachability | `:calls+` syntax | Two-rule recursive predicate |
| Path enumeration with aggregation | Awkward (SPARQL doesn't compose recursion + aggregation well) | Natural (`shortest(path)`, `count`, `max` work inside recursion) |
| Storage overhead | High (one triple per attribute) | Low (one row per entity) |
| Schema flexibility | Open-world: any subject can carry any property | Closed-world: each row matches the relation's column spec |
| Substring/text queries on attributes | `FILTER(CONTAINS(...))` works but isn't optimized | `str_includes` plus an index on the text column |

The honest summary: **Datalog encodes the same information more compactly and gives better query composability, at the cost of more upfront schema design.** RDF gives total schema flexibility at the cost of much more verbose data and weaker compositional queries.

---

## Why this matters for Virgil

For Virgil specifically:

- You control the schema (you parse the code; you decide what to store).
- The schema is relatively stable — it's a code model, not a Wild West knowledge graph.
- Detection rules need to be compositional (audit checks chain together).
- You want recursive aggregation (call depth, dependency count, max complexity).

Datalog is the right model. The remaining question is which Datalog engine has a future, since CozoDB's upstream has stalled. Options on the table:

1. **Stay on Cozo for now**, plan a migration once a clear successor emerges.
2. **Build a minimal Datalog engine on top of Fjall** — focused subset (recursion + negation + stratified aggregation, no time travel), which gives full control over the one piece of infrastructure currently rotting underneath the product.
3. **Move the data model to DuckDB + DuckPGQ**, accepting that the query language becomes SQL with graph extensions rather than Datalog. Compositional power survives in CTEs; recursive aggregation gets harder.

The data model in this document is portable across all three options. Schema design pays off regardless of which engine you land on.
