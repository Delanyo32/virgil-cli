# Language attributes — Java

Java-specific attributes that don't fit cleanly into the common
`symbol` columns. One row per Java symbol that has at least one
non-default value; symbols with all defaults MAY be omitted to keep the
table sparse.

## Schema

```
:create java_attrs {
    symbol_id: String =>
    annotations: [String] default [],
    is_final: Bool default false,
    is_synchronized: Bool default false,
    is_native: Bool default false,
    is_default: Bool default false,
    throws_clause: [String] default [],
    type_parameters: [String] default [],
}
```

| Column | Applies to | Source |
|---|---|---|
| `annotations` | every kind | leading `@Annotation` markers in `modifiers` node |
| `is_final` | class, method, field, parameter, local | `final` keyword in `modifiers` |
| `is_synchronized` | method | `synchronized` keyword in `modifiers` (not the statement) |
| `is_native` | method | `native` keyword in `modifiers` |
| `is_default` | method (in interface) | `default` keyword in `modifiers` |
| `throws_clause` | method, constructor | `throws` clause raw textual exception names |
| `type_parameters` | class, interface, method | `<T, U extends X>` type parameter list |

Rows for `symbol.kind` values not listed in the "Applies to" column
should still be emitted with default values for those columns; the
schema does not gate which symbol kinds may populate which fields.

## Extraction rules

### `annotations`

- AST source: walk the `modifiers` node and collect every
  `marker_annotation` (`@Service`) and `annotation` (`@Cacheable(value = "products")`)
  child.
- Value for each entry: the **simple name** of the annotation (the
  `name` field of the `marker_annotation` / `annotation` node), not its
  arguments and not its package-qualified form. `@Service` →
  `"Service"`. `@org.springframework.stereotype.Service` →
  `"Service"`. Annotation arguments are discarded.
- Order: preserve source order. A symbol with `@Service` followed by
  `@Transactional` produces `["Service", "Transactional"]`.
- An `annotation_type_declaration` itself (`public @interface Foo`)
  emits `kind: "interface"` per the existing extractor; its own
  declaration is not in its own `annotations` list (would be
  self-referential).
- Annotations attached to a *type* (`@NonNull String name`) are
  stripped from the `type` row's `display_name` (per `types-java.md`)
  and do **not** populate `annotations` on the enclosing symbol —
  type-use annotations have no schema slot in this contract.

### `is_final`

- AST source: the literal token `final` appears as a child of the
  symbol's `modifiers` node. `public static final int FOO = 1;` →
  `is_final = true`.
- Defaults to `false` when no `final` keyword is present.
- A method parameter declared `final` (`void f(final int x)`) sets
  `is_final = true` on the parameter symbol. Currently the Java
  extractor does not emit parameter symbols (see `references-java.md`);
  when parameter symbols land, this column applies.
- For records (`record Point(int x, int y)`), record components are
  implicitly final but the contract emits `is_final = false` for them
  unless `final` appears explicitly. Rationale: surface what the source
  said, not what the JLS infers. An audit query can join `symbol.kind`
  = `"class"` for records and treat their components as final.

### `is_synchronized`

- AST source: `synchronized` keyword as a child of a method's
  `modifiers` node. Only the **method modifier** form counts.
- `synchronized (x) { … }` *statements* (as in
  `RateLimitFilter.doFilterInternal` line 46) do **not** affect this
  column — they're not symbol modifiers, they're block statements.
- Defaults to `false`.

### `is_native` and `is_default`

- `is_native`: the `native` keyword on a method. Used for JNI bindings;
  no instances in the spring-api benchmark.
- `is_default`: the `default` keyword on an *interface* method body
  (`default void foo() { … }`). Distinct from annotation `default`
  values; the tree-sitter node kind is `default` keyword inside method
  modifiers.

### `throws_clause`

- AST source: the `throws` clause on a `method_declaration` or
  `constructor_declaration`. Tree-sitter exposes this as a `throws`
  node containing one or more type nodes.
- Value: list of the **textual** type names as they appear in source,
  one entry per declared exception. `throws ServletException, IOException`
  → `["ServletException", "IOException"]`.
- Generic exception types preserve their argument list:
  `throws MyException<T>` → `["MyException<T>"]`. Whitespace is
  normalised the same way as `type.display_name`.
- Defaults to `[]` for methods with no `throws` clause and for symbols
  that aren't methods/constructors.

#### `throws_clause` vs the `throws` relation — ambiguity

There are now two ways to ask "what exceptions does method M declare?":

1. `throws { function_id, exception_type_id }` — joins through `type`
   to get the resolved exception (`canonical_name`).
2. `java_attrs { symbol_id: M, throws_clause: [...] }` — raw textual
   list, no resolution.

**Decision: keep both, and define the split as follows.**

- The `throws` relation is the authoritative source for cross-language,
  resolved exception analysis. It joins with `type.canonical_name`. Use
  it whenever you want to filter by exception class (e.g. "every method
  that throws `IOException` transitively").
- `java_attrs.throws_clause` is the unresolved, textual view. Use it
  when:
  - You don't want a `type`-table join.
  - The exception type is unresolvable (third-party class not indexed)
    — `throws` will silently omit it; `throws_clause` will list it.
  - You want to preserve source-textual fidelity (e.g. detect a method
    that declares `throws Exception` *literally*, not via subclass).

Concrete rule: **every entry in `throws_clause` MUST have a
corresponding `throws` row when the exception type resolves**, and MAY
have one when it doesn't. The two are not allowed to diverge for
resolved exceptions. CI tests should assert this invariant.

The rationale for redundancy: the `throws` relation requires the
`type`-table dedup logic (per-file `type` rows), which makes
unresolved-exception queries clunky. The textual list is a constant-
time field lookup and is the right tool for raw-text checks. The cost
is one short list per method.

### `type_parameters`

- AST source: the `type_parameters` node directly under a
  `class_declaration` / `interface_declaration` / `method_declaration`.
- Value: list of textual parameter declarations, one entry per
  parameter, **including bounds**. `<T, U extends Comparable<U>>` →
  `["T", "U extends Comparable<U>"]`. Whitespace is normalised the
  same way as `type.display_name`.
- Defaults to `[]`.
- The type parameter names also produce `symbol` rows of kind
  `"variable"` (or a future `"type_parameter"` kind if added). The
  contract does not require that, but does require that references to
  `T` inside the body resolve to *something* — currently they're
  treated as unresolved and skipped (per `types-java.md` Resolution).

### Edge cases

- **Records with annotations on record components.** The annotation
  attaches to the synthetic accessor + field; the contract stores it
  on the `field` symbol's `annotations` list, not on the record class.
- **Annotations on a class declaration that wraps the class in
  `@interface`** (annotation type itself) populate `annotations` on the
  annotation-type's `symbol` row, same as any other class.
- **Inherited `@Override`** is a real source annotation
  (`@Override`) — it goes in `annotations` like any other.
- **Multiple modifiers in arbitrary order.** Tree-sitter normalises
  child order to source order; the extractor walks `modifiers` linearly
  so the keyword set is order-independent.
- **`abstract` and `static`.** These already populate
  `symbol.is_abstract` and `symbol.is_static` per the core schema and
  do **not** appear in `java_attrs`. Don't double-store.
- **`public` / `private` / `protected`.** These populate
  `symbol.visibility`. Don't double-store.

## Worked examples

All citations are 1-indexed line numbers into
`../virgil-skills/benchmarks/java/spring-api/`. `symbol_id` follows
ADR-0002 (`path|start_line|start_col|name|kind`); long paths are
abbreviated `…` in the tables.

### Example 1 — class with Spring annotation

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:19-20`

```java
@Service
public class ProductService {
```

Symbol: `…/ProductService.java|20|13|ProductService|class`.

`java_attrs` row:

| column | value |
|---|---|
| `symbol_id` | `…/ProductService.java\|20\|13\|ProductService\|class` |
| `annotations` | `["Service"]` |
| `is_final` | `false` |
| `is_synchronized` | `false` |
| `is_native` | `false` |
| `is_default` | `false` |
| `throws_clause` | `[]` |
| `type_parameters` | `[]` |

The `@Service` annotation lives at line 19; tree-sitter attaches it to
the `class_declaration`'s `modifiers` child. The `symbol_id` itself
uses the class keyword line (`20`), per the existing
`tree-sitter-java` symbol-extraction in
`src/languages/java/queries.rs` (the `name` field's position).

### Example 2 — method with `@Cacheable` and a complex value argument

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:25-26`

```java
@Cacheable(value = "products", key = "#category + '-' + #search + '-' + #page + '-' + #size")
public List<ProductDTO> findAll(String category, String search, int page, int size) {
```

Symbol: `…/ProductService.java|26|28|findAll|method`.

`java_attrs` row:

| column | value |
|---|---|
| `annotations` | `["Cacheable"]` |
| `is_final` | `false` |
| `is_synchronized` | `false` |
| `throws_clause` | `[]` |
| `type_parameters` | `[]` |

The annotation arguments (`value = "products"`, `key = "…"`) are
**dropped** — only the simple name `Cacheable` is recorded. Auditors
who need to query against annotation arguments must go to the
tree-sitter AST directly; the contract does not commit to surfacing
them.

### Example 3 — controller with two annotations

**Source.** `src/main/java/com/example/inventory/controller/ProductController.java:16-18`

```java
@RestController
@RequestMapping("/api/products")
public class ProductController {
```

Symbol: `…/ProductController.java|18|13|ProductController|class`.

`java_attrs` row:

| column | value |
|---|---|
| `annotations` | `["RestController", "RequestMapping"]` |
| `is_final` | `false` |
| `throws_clause` | `[]` |
| `type_parameters` | `[]` |

Order preserved.

### Example 4 — method with `throws` clause

**Source.** `src/main/java/com/example/inventory/middleware/AuthFilter.java:34-38`

```java
@Override
protected void doFilterInternal(HttpServletRequest request,
                                HttpServletResponse response,
                                FilterChain filterChain)
        throws ServletException, IOException {
```

Symbol: `…/AuthFilter.java|35|22|doFilterInternal|method`.

`java_attrs` row:

| column | value |
|---|---|
| `annotations` | `["Override"]` |
| `is_final` | `false` |
| `is_synchronized` | `false` |
| `throws_clause` | `["ServletException", "IOException"]` |
| `type_parameters` | `[]` |

Cross-check: the `throws` relation (see `types-java.md` Example 7) has
two rows pointing at the resolved `type` rows for `ServletException`
and `IOException`. The textual entries in `throws_clause` match those
two `type.display_name` values exactly. This is the redundancy the
contract preserves.

### Example 5 — static `final` field constant

**Source.** `src/main/java/com/example/inventory/middleware/RateLimitFilter.java:23`

```java
private static final int MAX_REQUESTS_PER_WINDOW = 100;
```

Symbol: `…/RateLimitFilter.java|23|29|MAX_REQUESTS_PER_WINDOW|variable`.

`java_attrs` row:

| column | value |
|---|---|
| `annotations` | `[]` |
| `is_final` | `true` |
| `is_synchronized` | `false` |
| `throws_clause` | `[]` |
| `type_parameters` | `[]` |

Note: `static` is **not** in `java_attrs` — it's already on
`symbol.is_static` per the core schema. Same for `private`
(`symbol.visibility = "private"`).

### Example 6 — Field with `@Autowired` and no explicit modifier set

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:22-23`

```java
@Autowired
private ProductRepository productRepository;
```

Symbol: `…/ProductService.java|23|30|productRepository|variable`.

`java_attrs` row:

| column | value |
|---|---|
| `annotations` | `["Autowired"]` |
| `is_final` | `false` |
| `throws_clause` | `[]` |
| `type_parameters` | `[]` |

This is the case where a sparse extension row is required *only*
because of one non-default field. The contract permits omitting rows
where every field is at default; if implementers prefer to always emit
a row, that's fine too — query callers must not rely on row presence
as a signal.

### Example 7 — Synchronized method (forward-looking; corpus uses statement form)

The spring-api corpus does not contain `synchronized` method
modifiers (it uses `synchronized (count) { … }` statements). The
contract for a hypothetical method:

```java
public synchronized void incrementCounter() { ... }
```

Symbol: `…|incrementCounter|method`.

`java_attrs` row:

| column | value |
|---|---|
| `is_synchronized` | `true` |

`RateLimitFilter.java:46`'s `synchronized (count) { ... }` block does
**not** populate this attribute — it's a statement, not a modifier.
