# Types — Java

Per [ADR-0003](adr/0003-level-3-types-and-references.md), every Java type
expression encountered in a parameter list, return-type position, field
declaration, cast target, throws clause, instanceof target, generic
argument, or `catch` parameter produces exactly one `type` row, deduped
per file by `display_name`.

## Tree-sitter node kinds

The `tree-sitter-java` grammar exposes the following type-expression
nodes. Every node listed here MUST be recognised by the extractor; nodes
not listed MUST NOT emit a `type` row.

| Node kind | What it represents | `type.kind` |
|---|---|---|
| `integral_type` | `byte`, `short`, `int`, `long`, `char` | `primitive` |
| `floating_point_type` | `float`, `double` | `primitive` |
| `boolean_type` | `boolean` | `primitive` |
| `void_type` | `void` (return position only) | `primitive` |
| `type_identifier` | bare reference type (`String`, `Product`, `T`) | `named` |
| `scoped_type_identifier` | qualified reference (`java.util.List`, `Map.Entry`) | `named` |
| `generic_type` | parameterised reference (`List<String>`, `Map<K,V>`) | `generic` |
| `array_type` | `int[]`, `String[][]`, `T[]` | `array` |
| `wildcard` (inside `type_arguments`) | `?`, `? extends T`, `? super T` | `named` (see below) |
| `intersection_type` | `<T extends A & B>` upper bound | `intersection` |
| `union_type` | multi-catch `catch (A | B e)` | `union` |

### Context-dependent splits

- `type_identifier` is always `named`; a name that happens to be a generic
  *parameter* of the enclosing class/method (`T`, `E`) is still emitted
  as `kind = "named"` but with `canonical_name = null` (see resolution).
- `wildcard` does not get its own kind. The wildcard expression is
  serialised into the parent `generic_type`'s `display_name` and only
  the parent emits a row. A bare wildcard outside `type_arguments` is a
  parse error in Java and never reaches the extractor.
- `void_type` only ever appears in `returns_type` rows; field/parameter
  positions reject `void` syntactically.
- The `dimensions` suffix on a parameter (`String args[]`) is folded
  into the parameter's `array_type` row — there is no separate
  `dimensions` node row.

## `display_name` construction

`display_name` is the textual rendering of the node, normalised by these
rules (applied in order):

1. Take the tree-sitter node's UTF-8 source slice.
2. Collapse any run of ASCII whitespace (`[ \t\r\n]+`) inside the slice
   to a single space.
3. Strip whitespace immediately inside angle brackets, square brackets,
   and around commas: `Map< K , V >` → `Map<K, V>`, `int [ ]` → `int[]`.
4. Preserve a single space after every comma in a `type_arguments` list.
5. Preserve a single space around `extends`, `super`, and `&` inside
   wildcards / intersection bounds.
6. Annotations on a type (e.g. `@NonNull String`) are **stripped** from
   `display_name`. Annotations on the *symbol* live in `java_attrs`.

`display_name` round-trips intent: `List<String>` and `List< String >`
produce the same value (`List<String>`); fully-qualified vs. simple
names (`java.util.List<String>` vs. `List<String>`) are **distinct**
display names — qualification is a syntactic choice the contract
preserves.

Examples:

| Source | `display_name` |
|---|---|
| `int` | `int` |
| `String` | `String` |
| `java.util.List` | `java.util.List` |
| `List<Product>` | `List<Product>` |
| `Map<String, BigDecimal>` | `Map<String, BigDecimal>` |
| `Optional<? extends Product>` | `Optional<? extends Product>` |
| `int[]` | `int[]` |
| `String[][]` | `String[][]` |
| `Map.Entry<Long, Integer>` | `Map.Entry<Long, Integer>` |
| `ServletException \| IOException` (multi-catch) | `ServletException \| IOException` |

## `canonical_name` resolution

The extractor MUST resolve `canonical_name` to the fully-qualified Java
binary name (dot-separated) whenever a deterministic answer exists. The
scope walk is:

1. **Same compilation unit.** If the simple name matches a top-level
   type declared in the same file, prepend the file's `package`
   declaration. Inner/nested types use `Outer.Inner` (dot-separated, no
   `$`).
2. **Explicit single-type imports.** A `type_identifier` matching the
   final segment of an `import x.y.Z;` resolves to `x.y.Z`.
3. **Static-member imports.** `import static x.y.Z.METHOD;` does **not**
   contribute to type resolution; it only contributes to identifier
   resolution (covered in `references-java.md`).
4. **On-demand imports (`import x.y.*;`).** If exactly one indexed file
   under package `x.y` declares a top-level type matching the simple
   name, resolve to `x.y.<Name>`. If zero or multiple candidates exist,
   `canonical_name = null`.
5. **Same package.** If another file in the same `package` declares a
   top-level type matching the simple name, resolve to
   `<package>.<Name>`.
6. **`java.lang` prelude.** A whitelist of names (`String`, `Object`,
   `Integer`, `Long`, `Boolean`, `Character`, `Byte`, `Short`, `Float`,
   `Double`, `Number`, `Math`, `System`, `Thread`, `Throwable`,
   `Exception`, `RuntimeException`, `Error`, `Iterable`, `Comparable`,
   `CharSequence`, `Class`, `Enum`, `Void`, `Runnable`, `Process`,
   `ProcessBuilder`) resolves to `java.lang.<Name>`.
7. **Otherwise** → `canonical_name = null`.

Counts as **unresolved** (`canonical_name = null`):

- Type parameters (`T`, `E`, `K`, `V` declared in the enclosing
  class/method's type-parameter list).
- Types from JARs that virgil-cli has not indexed (e.g. anything in
  `org.springframework.*` when only the benchmark sources are loaded).
- A bare simple name that matches **multiple** on-demand imports.
- Compile errors / partial parses.

### Generics and arrays

- `kind = "generic"` rows resolve `canonical_name` to the **raw**
  qualified name with the same argument list (e.g. `List<Product>`
  resolves to `java.util.List<com.example.inventory.model.Product>`
  when all components are resolvable). If any component is unresolvable,
  the whole row's `canonical_name` is `null`.
- `kind = "array"` rows resolve to `<element-canonical>[]` (one `[]`
  per dimension). `int[]` resolves to `int[]`; `Product[]` resolves to
  `com.example.inventory.model.Product[]`.
- `kind = "intersection"` rows resolve to `<lhs-canonical> & <rhs-canonical>`
  with each side resolved independently; if any side is unresolvable
  the whole row is `null`.
- `kind = "union"` rows (multi-catch) resolve the same way.

### Primitives

`int`, `boolean`, `void`, etc. resolve their `canonical_name` to the
keyword itself (no qualification — primitives have no package). This is
explicit so that downstream joins on `canonical_name` treat
`primitive`-kind rows uniformly with `named`-kind rows.

### Aliases

Java has no `type` aliases (no equivalent of Rust's `type Foo = Bar;`).
The contract therefore has nothing to say about alias resolution;
import aliasing also does not exist (`import a.b.C as D` is invalid).

## Identity

Per [ADR-0003](adr/0003-level-3-types-and-references.md):

```
type.id = blake3("java" | file_id | display_name)
```

`display_name` is the post-normalisation string defined above.
Two distinct source spellings that normalise to the same `display_name`
in the same file produce a single `type` row.

Two files with the same `display_name` (e.g. both contain `String`)
produce distinct `type` rows — dedup is per-file, cross-file aggregation
joins through `canonical_name`.

`language` on every Java `type` row is the literal string `"java"`.

---

## Field types — `field_type` relation

Per the schema, every Java field declaration (class field, enum
field, record component) with a typed declarator emits a
`field_type {symbol_id, type_id}` row linking the field symbol to
its `type` row. Local variables and method parameters are not
fields and use `parameter` / `references` wiring instead.

---

## Worked examples

All citations are line:col ranges into files under
`../virgil-skills/benchmarks/java/spring-api/`. `start_byte` /
`start_line` / `start_col` are the tree-sitter `Range` of the type-expression
node itself, not the enclosing parameter / field.

### Example 1 — primitive in parameter (`int`)

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:26`

```java
public List<ProductDTO> findAll(String category, String search, int page, int size) {
```

The `int` at `page` is a `integral_type` node.

**`type` row.**

| column | value |
|---|---|
| `id` | `blake3("java" \| "src/main/java/com/example/inventory/service/ProductService.java" \| "int")` |
| `kind` | `"primitive"` |
| `language` | `"java"` |
| `display_name` | `"int"` |
| `canonical_name` | `"int"` |

(The two `int` occurrences on this line dedup to a single row — same
file, same `display_name`.)

**Referencing rows.**

- `parameter{function_id: "<…ProductService.java>|26|4|findAll|method", index: 2, name: "page", type_id: <above>, is_optional: false, has_default: false}`
- `parameter{function_id: "<…ProductService.java>|26|4|findAll|method", index: 3, name: "size", type_id: <above>, is_optional: false, has_default: false}`

### Example 2 — named, java.lang prelude (`String`)

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:26`

The two `String` parameter types (`category`, `search`).

**`type` row.**

| column | value |
|---|---|
| `id` | `blake3("java" \| "<…ProductService.java>" \| "String")` |
| `kind` | `"named"` |
| `language` | `"java"` |
| `display_name` | `"String"` |
| `canonical_name` | `"java.lang.String"` (prelude rule) |

**Referencing rows.**

- `parameter{function_id: "…|findAll|method", index: 0, name: "category", type_id: <above>}`
- `parameter{function_id: "…|findAll|method", index: 1, name: "search", type_id: <above>}`

Other `String` occurrences in the same file (line 59 `"ACTIVE"` literal
is *not* a type; line 21 `productRepository` field type is `ProductRepository`,
not String) reuse this same row.

### Example 3 — generic (`List<ProductDTO>`)

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:26`

```java
public List<ProductDTO> findAll(String category, String search, int page, int size) {
```

The return-type node is a `generic_type` containing `type_identifier`
`List` and a `type_arguments` of `type_identifier` `ProductDTO`.

**Three `type` rows are emitted, all in this file:**

Row A (the parent `generic_type`):

| column | value |
|---|---|
| `id` | `blake3("java" \| file_id \| "List<ProductDTO>")` |
| `kind` | `"generic"` |
| `display_name` | `"List<ProductDTO>"` |
| `canonical_name` | `"java.util.List<com.example.inventory.dto.ProductDTO>"` |

`List` resolves via `import java.util.List;` (line 16). `ProductDTO`
resolves via `import com.example.inventory.dto.ProductDTO;` (line 3).

Row B (the inner `List` identifier):

| column | value |
|---|---|
| `kind` | `"named"` |
| `display_name` | `"List"` |
| `canonical_name` | `"java.util.List"` |

Row C (the inner `ProductDTO` identifier):

| column | value |
|---|---|
| `kind` | `"named"` |
| `display_name` | `"ProductDTO"` |
| `canonical_name` | `"com.example.inventory.dto.ProductDTO"` |

**Referencing rows.**

- `returns_type{function_id: "…|findAll|method", type_id: <Row A id>}`

The two component rows are linked from this row by the extractor only
implicitly through `display_name` parsing; nothing in the schema joins
generic components back to their parent type.

### Example 4 — array (`String[]`-style via `split`)

**Source.** `src/main/java/com/example/inventory/middleware/RateLimitFilter.java:69`

```java
return xForwardedFor.split(",")[0].trim();
```

This is a method return value, not a type position — so it does **not**
emit a `type` row. Use this counter-example to anchor: `[0]` in an
expression context is an `array_access`, not an `array_type`.

The benchmark's array-type cases live in lambda parameters and varargs.
The closest direct example is in `RateLimitFilter.java:43`:

```java
RequestCount count = requestCounts.computeIfAbsent(clientIp, k -> new RequestCount());
```

No array there either. A reliable array example:

**Source.** `src/main/java/com/example/inventory/middleware/AuthFilter.java:55`

```java
String token = authHeader.substring(7);
```

Local variable declaration — `String` is `type_identifier`, no array.

The benchmark corpus does not contain an explicit `T[]` type
declaration in our scanned files. **The contract still binds the
extractor:** whenever a future Java file declares e.g. `byte[] bytes`,
the row emitted is:

| column | value |
|---|---|
| `kind` | `"array"` |
| `display_name` | `"byte[]"` |
| `canonical_name` | `"byte[]"` |

with element kind `primitive`. Multi-dimensional `String[][]` →
`display_name = "String[][]"`, `canonical_name = "java.lang.String[][]"`.

### Example 5 — qualified scoped type (`java.math.BigDecimal`)

**Source.** `src/main/java/com/example/inventory/util/PriceCalculator.java:23`

```java
public BigDecimal calculateDiscount(BigDecimal orderTotal, String userTier,
                                    int itemCount, boolean hasPromoCode,
                                    boolean isFirstOrder, String season) {
```

The return type `BigDecimal` is a bare `type_identifier`, resolved via
`import java.math.BigDecimal;` (line 3 of the file).

**`type` row.**

| column | value |
|---|---|
| `kind` | `"named"` |
| `display_name` | `"BigDecimal"` |
| `canonical_name` | `"java.math.BigDecimal"` |

**Also worth noting:** `boolean` (lines 25–26) emits:

| column | value |
|---|---|
| `kind` | `"primitive"` |
| `display_name` | `"boolean"` |
| `canonical_name` | `"boolean"` |

…and the four `boolean` parameter occurrences dedup to one row.

### Example 6 — multi-catch union (`SignatureException | Exception` is two single-catch blocks, not a union here)

**Source.** `src/main/java/com/example/inventory/middleware/AuthFilter.java:67-73`

```java
} catch (SignatureException e) {
    response.setStatus(HttpServletResponse.SC_UNAUTHORIZED);
    ...
} catch (Exception e) {
    ...
}
```

The corpus uses two sequential `catch` clauses, **not** a multi-catch
`union_type`. Each clause's parameter emits a separate `named` row.

Two `type` rows:

| column | row 1 | row 2 |
|---|---|---|
| `kind` | `"named"` | `"named"` |
| `display_name` | `"SignatureException"` | `"Exception"` |
| `canonical_name` | `"io.jsonwebtoken.SignatureException"` (via import line 5) | `"java.lang.Exception"` (prelude) |

**The contract for an actual multi-catch** (e.g. if AuthFilter were
rewritten to `catch (SignatureException | RuntimeException e)`): the
extractor MUST emit a single `union` row with
`display_name = "SignatureException | RuntimeException"` (spaces around
`|`) and `canonical_name` resolved component-wise when both sides
resolve.

### Example 7 — `throws`-clause types

**Source.** `src/main/java/com/example/inventory/middleware/AuthFilter.java:34-38`

```java
@Override
protected void doFilterInternal(HttpServletRequest request,
                                HttpServletResponse response,
                                FilterChain filterChain)
        throws ServletException, IOException {
```

Two `type` rows emitted from the `throws` clause:

| column | row A | row B |
|---|---|---|
| `kind` | `"named"` | `"named"` |
| `display_name` | `"ServletException"` | `"IOException"` |
| `canonical_name` | `"javax.servlet.ServletException"` (via import line 7) | `"java.io.IOException"` (via import line 12) |

**Referencing rows.**

- `throws{function_id: "<…AuthFilter.java>|35|14|doFilterInternal|method", exception_type_id: <row A id>}`
- `throws{function_id: "…", exception_type_id: <row B id>}`

The same two rows are referenced from
`src/main/java/com/example/inventory/middleware/CacheInterceptor.java:28,53`
when those methods declare `throws Exception` — but `Exception` is a
different `display_name` so it becomes a third row keyed in its own
file. (Reminder: dedup is per-file.)

Note the overlap with `java_attrs.throws_clause`: see
`attrs-java.md` for the explicit split between the `throws` relation
(canonical, resolved) and the `throws_clause` attribute (raw textual
list, for queries that don't want to join through `type`).
