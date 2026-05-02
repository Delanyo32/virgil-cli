# virgil-cli DSL Reference

virgil-cli ships two composable JSON DSLs:

1. **Query DSL** — passed to `virgil projects query` to find symbols, traverse the call graph, or read source ranges.
2. **Audit Pipeline DSL** — declarative graph-stage pipelines that drive `virgil audit` and can be embedded in queries via the `graph` field.

Both DSLs are pure JSON, so they can be written inline (`--q '...'`), loaded from a file (`--file`), or stored as built-in / project-local rules.

---

## Table of Contents

- [Query DSL](#query-dsl)
  - [Field Reference](#field-reference)
  - [Name Filters](#name-filters)
  - [`has` — Doc / Decorator Filter](#has--doc--decorator-filter)
  - [Symbol Kinds (`find`)](#symbol-kinds-find)
  - [Call Graph Traversal](#call-graph-traversal)
  - [Reading Files (`read`)](#reading-files-read)
  - [Embedded Audit (`graph`)](#embedded-audit-graph)
  - [Output Formats](#output-formats)
  - [Query Examples](#query-examples)
- [Audit Pipeline DSL](#audit-pipeline-dsl)
  - [File Shape](#file-shape)
  - [Stage Reference](#stage-reference)
    - [`select`](#select)
    - [`match_pattern`](#match_pattern)
    - [`compute_metric`](#compute_metric)
    - [`group_by`](#group_by)
    - [`count`](#count)
    - [`ratio`](#ratio)
    - [`max_depth`](#max_depth)
    - [`find_cycles`](#find_cycles)
    - [`find_duplicates`](#find_duplicates)
    - [`taint_sources` / `taint_sanitizers` / `taint_sinks`](#taint-sources--sanitizers--sinks)
    - [`flag`](#flag)
  - [`WhereClause`](#whereclause)
  - [`NumericPredicate`](#numericpredicate)
  - [Edge Kinds](#edge-kinds)
  - [Node Types](#node-types)
  - [Severity Resolution](#severity-resolution)
  - [Message Interpolation](#message-interpolation)
  - [Pipeline Discovery Order](#pipeline-discovery-order)
  - [Pipeline Examples](#pipeline-examples)

---

# Query DSL

A query is a single JSON object. All fields are optional and AND-composed. The shape:

```json
{
  "files": "src/api/**",
  "files_exclude": ["**/test/**", "**/node_modules/**"],
  "find": "function",
  "name": {"regex": "^handle[A-Z]"},
  "visibility": "exported",
  "inside": "AuthService",
  "has": "@deprecated",
  "lines": {"min": 10, "max": 100},
  "body": false,
  "preview": 5,
  "calls": "down",
  "depth": 2,
  "format": "snippet",
  "read": null,
  "graph": null
}
```

## Field Reference

| Field | Type | Description |
|---|---|---|
| `files` | `string` \| `[string]` | Glob(s) restricting which files are parsed (e.g. `"src/**/*.ts"`). |
| `files_exclude` | `[string]` | Globs to subtract from the file set. |
| `find` | `string` \| `[string]` | Symbol kind(s) to match. See [Symbol Kinds](#symbol-kinds-find). |
| `name` | `string` \| `{contains}` \| `{regex}` | Name filter. See [Name Filters](#name-filters). |
| `visibility` | `string` | `exported`, `public`, `private`, `protected`, `internal`. |
| `inside` | `string` | Only return symbols whose enclosing parent has this name (line-range containment, not AST). |
| `has` | `string` \| `[string]` \| `{not: string}` | Filter by associated comment / decorator text. See [`has`](#has--doc--decorator-filter). |
| `lines` | `{min, max}` | Line-count range. |
| `body` | `bool` | Include full source body in each result. |
| `preview` | `number` | Number of preview lines to return per result. |
| `calls` | `string` | Call-graph traversal direction: `down`, `up`, `both`. |
| `depth` | `number` | Call-graph BFS depth. Default 1, max 5. |
| `format` | `string` | Override `--out`. See [Output Formats](#output-formats). |
| `read` | `string` | Path to read raw content from (bypasses symbol extraction). Combine with `lines: {min, max}` for a range read. |
| `graph` | `[stages]` | Embedded audit pipeline run after symbol filtering. See [Audit Pipeline DSL](#audit-pipeline-dsl). |

## Name Filters

```json
{ "name": "handle*" }                  // glob
{ "name": { "contains": "auth" } }     // substring (case-sensitive)
{ "name": { "regex": "^get[A-Z]" } }   // Rust regex
```

## `has` — Doc / Decorator Filter

```json
{ "has": "@deprecated" }                              // single string
{ "has": ["@deprecated", "TODO"] }                    // any of these
{ "has": { "not": "docstring" } }                     // inverse: symbols WITHOUT a docstring
```

The matcher checks doc comments, attributes, and decorators attached to the symbol.

## Symbol Kinds (`find`)

| Kind | Notes |
|---|---|
| `function` | Includes `Function` and `ArrowFunction`. |
| `method` | Class / impl / interface methods. |
| `class` | Classes and class-like constructs. |
| `type` | Type aliases and C-style `typedef`s. |
| `enum`, `struct`, `trait`, `union` | As named. |
| `variable`, `constant`, `property` | Top-level / member declarations. |
| `namespace`, `module` | Module / namespace declarations. |
| `macro` | Macro declarations (Rust, C/C++). |
| `arrow_function` | Match only arrow functions (subset of `function`). |
| `constructor` | Methods named `constructor`, `__init__`, `__construct`, or `new`. |
| `import` | Import / `use` statements. The import `kind` field is a free-form string per language. |
| `any` | Any kind. |

## Call Graph Traversal

```json
{ "find": "function", "name": "login", "calls": "down", "depth": 3 }
```

- `down` — callees of matched symbols.
- `up` — callers of matched symbols.
- `both` — both directions.

Resolution is name-based via the `symbols_by_name` lookup. There is no type info, so polymorphic dispatch and indirect calls are best-effort.

## Reading Files (`read`)

```json
{ "read": "src/main.rs" }
{ "read": "src/main.rs", "lines": { "min": 10, "max": 60 } }
```

`read` bypasses the symbol pipeline and returns raw text. All other fields except `lines` are ignored.

## Embedded Audit (`graph`)

The `graph` field accepts the same stage list documented under [Audit Pipeline DSL](#audit-pipeline-dsl). It runs after the symbol filter, so the seed nodes are the symbols already matched by the rest of the query. This lets you ask audit-style questions scoped to a slice of the codebase:

```json
{
  "files": "src/auth/**",
  "find": ["function", "method"],
  "graph": [
    { "compute_metric": "cyclomatic_complexity" },
    { "flag": {
        "pattern": "high_complexity_in_auth",
        "message": "{{name}} has CC={{cyclomatic_complexity}}",
        "severity": "warning",
        "severity_map": [
          { "when": { "metrics": { "cyclomatic_complexity": { "gte": 15 } } }, "severity": "error" }
        ]
    } }
  ]
}
```

## Output Formats

`--out` (or `format` inside the query) controls the result shape. Every format is JSON.

| Format | Content |
|---|---|
| `outline` | name, kind, file, line, signature (default). |
| `snippet` | outline + preview lines + docstring. |
| `full` | outline + full body. |
| `tree` | hierarchical: file → class → methods. |
| `locations` | `file:line` strings only. |
| `summary` | counts grouped by kind and file. |

The wrapping envelope:

```json
{
  "project": "myapp",
  "query_ms": 42,
  "files_parsed": 8,
  "total": 3,
  "results": [ ... ]
}
```

## Query Examples

**Find every exported handler with no doc comment:**

```json
{
  "files": "src/api/**",
  "find": "function",
  "name": "handle*",
  "visibility": "exported",
  "has": { "not": "docstring" }
}
```

**Trace what `login` calls, two levels deep:**

```json
{ "find": "function", "name": "login", "calls": "down", "depth": 2 }
```

**List all callers of `unsafe_eval`:**

```json
{ "find": "any", "name": "unsafe_eval", "calls": "up", "depth": 3 }
```

**Read a slice of a file:**

```json
{ "read": "src/pipeline/executor.rs", "lines": { "min": 1, "max": 80 } }
```

**Long methods inside `AuthService`:**

```json
{
  "find": "method",
  "inside": "AuthService",
  "lines": { "min": 50 },
  "body": true
}
```

**Deprecated symbols anywhere in the project:**

```json
{ "find": "any", "has": "@deprecated", "format": "locations" }
```

---

# Audit Pipeline DSL

A pipeline is a JSON document describing one rule. Stages execute left-to-right; each stage reads from and writes to a shared list of `PipelineNode`s. A pipeline ending in `flag` emits findings; otherwise it produces a node list (useful inside a query's `graph`).

## File Shape

```json
{
  "pipeline": "my_rule",
  "category": "security",
  "description": "What this rule detects",
  "languages": ["rust", "typescript"],
  "graph": [ /* stages */ ]
}
```

| Field | Required | Notes |
|---|---|---|
| `pipeline` | yes | Unique pipeline name; reported on each finding and accepted by `--pipeline`. |
| `category` | yes | One of: `security`, `architecture`, `tech_debt`, `code_style`, `scalability`, `complexity`. |
| `description` | recommended | Free text. |
| `languages` | recommended | Array of language tags (`rust`, `typescript`, `javascript`, `c`, `cpp`, `csharp`, `python`, `go`, `java`, `php`). Empty / missing = all. |
| `graph` | yes | Ordered array of stages. |

## Stage Reference

### `select`

Seeds the pipeline with graph nodes of one type, optionally filtered.

```json
{
  "select": "symbol",
  "where":   { "kind": ["function", "method"] },
  "exclude": { "or": [ { "is_test_file": true }, { "is_generated": true } ] }
}
```

- `select` — `file`, `symbol`, or `call_site`.
- `where` — keep nodes matching this [`WhereClause`](#whereclause).
- `exclude` — drop nodes matching this clause.

### `match_pattern`

Runs a tree-sitter S-expression query against source files. Each capture becomes a node. An optional `when` post-filters captures.

```json
{
  "match_pattern": "(assignment_expression left: (member_expression object: (identifier) @obj)) @assign",
  "when": { "lhs_is_parameter": true }
}
```

`lhs_is_parameter` is the only `when` flag specific to `match_pattern`: it asserts the matched assignment's LHS object identifier is a named parameter of the enclosing function.

### `compute_metric`

Computes a named metric and writes it onto each node's `metrics` map. Subsequent stages can filter on it via `WhereClause.metrics` and interpolate it into messages.

```json
{ "compute_metric": "cyclomatic_complexity" }
```

Built-in metrics:

| Metric | Description |
|---|---|
| `cyclomatic_complexity` | Decision-point complexity. |
| `cognitive_complexity` | Sonar-style cognitive complexity. |
| `function_length` | Lines in the function body. |
| `nesting_depth` | Maximum block nesting. |
| `comment_to_code_ratio` | Per-file ratio. |
| `efferent_coupling` | Outgoing import count. |

### `group_by`

Tags each node with a `_group` value. Required input to `count`.

```json
{ "group_by": "file" }
```

Accepted keys: `file` (alias `file_path`), `language`, `kind`, `name`, or any metric produced upstream.

### `count`

Counts members per group and keeps groups passing the threshold. The count is exposed as the `count` metric on the surviving representative node.

```json
{ "count": { "threshold": { "gte": 30 } } }
```

### `ratio`

Computes `numerator / denominator` over the current node set, where both are filters expressed as `WhereClause`. Optionally drops results not matching `threshold`.

```json
{
  "ratio": {
    "numerator":   { "where": { "exported": true } },
    "denominator": {},
    "threshold":   { "metrics": { "ratio": { "gte": 0.8 } } }
  }
}
```

The result writes a `ratio` (float) and `total` (int) metric onto the surviving node(s).

### `max_depth`

Walks an edge type to find the longest chain rooted at each seed. Filters by depth.

```json
{
  "max_depth": {
    "edge": "imports",
    "skip_barrel_files": true,
    "threshold": { "gte": 4 }
  }
}
```

Writes `depth` and `cycle_path` (the chain). `skip_barrel_files` is used by import-depth audits to avoid penalising re-export hubs.

### `find_cycles`

Detects strongly-connected components on a given edge type (Tarjan SCC).

```json
{ "find_cycles": { "edge": "imports" } }
```

Writes `cycle_size` (int) and `cycle_path` (string) per cycle.

### `find_duplicates`

Groups nodes by a property and emits the duplicates.

```json
{ "find_duplicates": { "by": "body", "min_count": 2 } }
```

`by` accepts node properties such as `body`, `name`, or any metric. `min_count` defaults to 2.

### Taint: sources / sanitizers / sinks

Taint analysis is decomposed into three accumulating stages plus a normal `flag` to emit findings. Patterns are simple substring matches against call expressions / member accesses.

```json
[
  { "taint_sources": [
      { "pattern": "request.form",   "kind": "user_input" },
      { "pattern": "os.environ.get", "kind": "env_var"   }
  ] },
  { "taint_sanitizers": [
      { "pattern": "escape" },
      { "pattern": "quote"  }
  ] },
  { "taint_sinks": [
      { "pattern": "cursor.execute", "vulnerability": "sql_injection" }
  ] },
  { "flag": {
      "pattern": "sql_injection",
      "message": "Tainted data reaches {{name}} at {{file}}:{{line}}",
      "severity": "error"
  } }
]
```

The combined legacy form is still accepted and desugared automatically:

```json
{ "taint": { "sources": [...], "sanitizers": [...], "sinks": [...] } }
```

### `flag`

Emits an audit finding for each remaining node.

```json
{
  "flag": {
    "pattern": "oversized_module",
    "message": "Module `{{name}}` has {{count}} symbols",
    "severity": "warning",
    "severity_map": [
      { "when": { "metrics": { "count": { "gte": 60 } } }, "severity": "error"   },
      { "when": { "metrics": { "count": { "gte": 30 } } }, "severity": "warning" },
      { "severity": "info" }
    ],
    "pipeline_name": "oversized_module_override"
  }
}
```

| Field | Notes |
|---|---|
| `pattern` | Required. Reported verbatim as the finding's pattern. |
| `message` | Required. Supports `{{var}}` interpolation. |
| `severity` | Bare default. Used when no `severity_map` entry matches. |
| `severity_map` | Ordered list. First matching `when` wins. An entry with no `when` is the catch-all. |
| `pipeline_name` | Optional override of the pipeline's `pipeline` field for this finding. |

See [Severity Resolution](#severity-resolution) for the precise tiebreak rules.

## `WhereClause`

The composable predicate used by `select.where`, `select.exclude`, `match_pattern.when`, `severity_map[].when`, and `ratio` filters. An empty clause is always true.

| Field | Type | Notes |
|---|---|---|
| `and` / `or` / `not` | nested clause(s) | Logical composition. |
| `is_test_file` | `bool` | Path matches a test-file heuristic. |
| `is_generated` | `bool` | Path matches a generated-file heuristic. |
| `is_barrel_file` | `bool` | Re-export hub heuristic. |
| `is_nolint` | `bool` | *Reserved.* Source-comment suppression handled by the executor. |
| `exported` | `bool` | Node visibility. |
| `kind` | `[string]` | Symbol kind (case-insensitive). |
| `unreferenced` | `bool` | Node has no incoming references. |
| `is_entry_point` | `bool` | Node is a recognised entry point (e.g. `main`). |
| `lhs_is_parameter` | `bool` | `match_pattern` only — see [`match_pattern`](#match_pattern). |
| `metrics` | `{ name: NumericPredicate }` | Filter on any metric written by `compute_metric`. |

## `NumericPredicate`

Any subset of `gte`, `lte`, `gt`, `lt`, `eq`. All present predicates are AND-combined.

```json
{ "metrics": { "cyclomatic_complexity": { "gt": 10, "lt": 20 } } }
{ "metrics": { "count":                  { "eq": 0 } } }
```

An empty predicate matches every value.

## Edge Kinds

Used by `max_depth` and `find_cycles`:

`calls`, `imports`, `flows_to`, `acquires`, `released_by`, `contains`, `exports`, `defined_in`.

## Node Types

Used by `select`:

| Type | Description |
|---|---|
| `file` | One node per source file in the workspace. |
| `symbol` | One node per declared symbol. |
| `call_site` | One node per call expression. |

## Severity Resolution

Given a flag config:

1. If `severity_map` is absent → use `severity`, defaulting to `"warning"`.
2. If `severity_map` is present → walk entries in order; first matching `when` wins. An entry with `when` absent (or empty) is a catch-all.
3. If no entry matches and `severity` is set → use `severity`.
4. If no entry matches and no bare `severity` → the finding is **suppressed** entirely.

This last case is how rules expose progressive severities while leaving the "below the floor" range silent.

## Message Interpolation

`flag.message` supports `{{var}}` placeholders. Always available:

`{{name}}`, `{{kind}}`, `{{file}}`, `{{line}}`, `{{language}}`.

Plus every metric on the node, e.g. `{{cyclomatic_complexity}}`, `{{count}}`, `{{depth}}`, `{{cycle_size}}`, `{{cycle_path}}`, `{{edge_count}}`, `{{ratio}}`, `{{total}}`. Floats render with two decimal places.

## Pipeline Discovery Order

When `virgil audit` runs, pipelines are loaded from three locations (later locations are *added*, not overridden):

1. **Built-ins** — bundled in the binary at `src/audit/builtin/*.json` (~300 rules).
2. **Project-local** — `.virgil/pipelines/*.json` in the workspace root.
3. **User-global** — `~/.virgil/pipelines/*.json`.

A specific file can also be passed via `--file <path.json>`.

`--language`, `--category`, and `--pipeline` filter the resulting set.

## Pipeline Examples

### Cyclomatic complexity (graded severity)

```json
{
  "pipeline": "cyclomatic_complexity",
  "category": "complexity",
  "languages": ["rust", "typescript", "python"],
  "graph": [
    { "select": "symbol",
      "where":   { "kind": ["function", "method"] },
      "exclude": { "is_test_file": true } },
    { "compute_metric": "cyclomatic_complexity" },
    { "flag": {
        "pattern": "cyclomatic_complexity",
        "message": "`{{name}}` has CC={{cyclomatic_complexity}}",
        "severity_map": [
          { "when": { "metrics": { "cyclomatic_complexity": { "gte": 20 } } }, "severity": "error"   },
          { "when": { "metrics": { "cyclomatic_complexity": { "gt":  10 } } }, "severity": "warning" }
        ]
    } }
  ]
}
```

(No catch-all → functions with CC ≤ 10 produce no finding.)

### Circular imports

```json
{
  "pipeline": "circular_dependencies_rust",
  "category": "architecture",
  "languages": ["rust"],
  "graph": [
    { "select": "file",
      "exclude": { "or": [ { "is_test_file": true }, { "is_generated": true } ] } },
    { "find_cycles": { "edge": "imports" } },
    { "flag": {
        "pattern": "circular_dependency",
        "message": "Circular dependency ({{cycle_size}} files): {{cycle_path}}",
        "severity_map": [
          { "when": { "metrics": { "cycle_size": { "eq":  2 } } }, "severity": "info"    },
          { "when": { "metrics": { "cycle_size": { "lte": 5 } } }, "severity": "warning" },
          { "severity": "error" }
        ]
    } }
  ]
}
```

### Oversized module (group + count)

```json
{
  "pipeline": "oversized_module",
  "category": "architecture",
  "graph": [
    { "select": "symbol", "exclude": { "is_test_file": true } },
    { "group_by": "file" },
    { "count": { "threshold": { "gte": 30 } } },
    { "flag": {
        "pattern": "oversized_module",
        "message": "{{file}} declares {{count}} symbols",
        "severity": "warning"
    } }
  ]
}
```

### SQL injection (taint)

```json
{
  "pipeline": "sql_injection_python",
  "category": "security",
  "languages": ["python"],
  "graph": [
    { "taint_sources": [
        { "pattern": "request.form", "kind": "user_input" },
        { "pattern": "request.args", "kind": "user_input" },
        { "pattern": "input(",       "kind": "user_input" }
    ] },
    { "taint_sanitizers": [ { "pattern": "escape" }, { "pattern": "quote" } ] },
    { "taint_sinks": [
        { "pattern": "cursor.execute", "vulnerability": "sql_injection" },
        { "pattern": "db.execute",     "vulnerability": "sql_injection" }
    ] },
    { "flag": {
        "pattern": "sql_injection",
        "message": "Tainted input reaches SQL execution at {{file}}:{{line}}",
        "severity": "error"
    } }
  ]
}
```

### Excessive public API (ratio)

```json
{
  "pipeline": "excessive_public_api",
  "category": "architecture",
  "graph": [
    { "select": "symbol" },
    { "group_by": "file" },
    { "ratio": {
        "numerator":   { "where": { "exported": true } },
        "denominator": {},
        "threshold":   { "metrics": { "ratio": { "gte": 0.8 }, "total": { "gte": 20 } } }
    } },
    { "flag": {
        "pattern": "excessive_public_api",
        "message": "{{file}} exports {{ratio}} of its {{total}} symbols",
        "severity": "warning"
    } }
  ]
}
```

### Tree-sitter pattern: argument mutation

```json
{
  "pipeline": "argument_mutation_javascript",
  "category": "code_style",
  "languages": ["javascript", "typescript"],
  "graph": [
    { "match_pattern": "(assignment_expression left: (member_expression object: (identifier) @obj)) @assign",
      "when": { "lhs_is_parameter": true } },
    { "flag": {
        "pattern": "argument_mutation",
        "message": "Mutates parameter at {{file}}:{{line}}",
        "severity": "warning"
    } }
  ]
}
```

---

For hundreds more worked examples covering every supported language, browse `src/audit/builtin/*.json` in the repository.
