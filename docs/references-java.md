# References — Java

Per [ADR-0003](adr/0003-level-3-types-and-references.md), every
identifier occurrence in Java source gets a `references` row. The
four `ref_kind` values (`read`, `write`, `type_use`, `import_use`)
partition all occurrences. Updated per `docs/contract-review.md`
(policy 1): the schema now keys `references` by
`(referrer_id, site_file, site_start_byte, match_index)` with
`referent_id` nullable in the value position. Unresolvable
references emit a single row with `referent_id = null` (the
previous Java-specific "skip" behavior is replaced with the
cross-language null-row convention). `match_index = 0` for the
primary/only candidate; overload resolution emits additional rows
at `match_index = 1, 2, ...` sharing the same site.

## Lexical scope rules

Java has nested static scoping with four scope levels relevant to the
extractor:

1. **Block scope.** Every `{ … }` opens a new scope. `for`, `try`,
   `catch`, `if`, `while`, `switch`, lambda bodies, and the
   parenthesised header of a `for` statement (`for (int i = …)`) all
   introduce block scope.
2. **Method scope.** Method/constructor parameters and type parameters
   (`<T>`) live one level above the body block. Lambda parameters
   create their own method-like scope nested under the enclosing method.
3. **Class scope.** Fields, nested types, and methods declared in a
   `class` / `interface` / `enum` / `record` / `annotation_type_declaration`
   body. Static and instance members share the same name space for
   lookup; the resolver does not distinguish them.
4. **Compilation-unit (file) scope.** Top-level types declared in the
   file plus the `package` declaration's contents. Java has no
   first-class module scope below the package — all top-level types in
   a package are mutually visible.

### Lookup walk

For a bare `identifier` token at some source position, the resolver
walks outward in this order and **stops at the first match**:

1. Innermost enclosing block (locals, including catch parameters,
   for-loop variables, lambda parameters).
2. Each enclosing block, walking out to the method body.
3. Method parameters and method-level type parameters of the enclosing
   method / constructor / lambda.
4. Class scope of the enclosing type — instance fields, static fields,
   inner type names, methods of this class.
5. Class scope of each enclosing outer type (for nested classes), from
   innermost to outermost.
6. Supertypes of the enclosing type — fields and methods inherited from
   the `extends` chain and implemented `implements` interfaces, when
   those types are indexed. Inherited names resolve to the supertype's
   declaration `symbol_id`. If the supertype is not indexed, the
   identifier is unresolved.
7. File-level: other top-level types in the same file.
8. Single-type imports.
9. Same-package types.
10. On-demand `*` imports (only when exactly one candidate exists).
11. Static-member imports (`import static x.y.Z.METHOD;` makes
    `METHOD` resolvable as if locally bound at file scope).
12. `java.lang` prelude (whitelist in `types-java.md`).

### Shadowing

- **Inner binding wins.** A local variable named `count` shadows a
  field named `count` for the duration of the local's scope. The
  resolver emits `referent_id = <local symbol id>` for those
  occurrences and **only** the local id — the shadowed field is not
  recorded as a second match.
- An access prefixed with `this.` always bypasses local-variable
  shadowing and resolves to the field on the enclosing instance type.
- An access prefixed with `super.` resolves against the direct
  superclass's scope, skipping the enclosing class.
- A `for`-loop header variable shadows any same-named variable in the
  enclosing block; the loop body sees the loop variable.
- Method parameters shadow class fields. The body's bare `count`
  resolves to the parameter.

### Module-qualified names

Java does not use `::` like C++; qualification is always dot-separated
and parsed as either `field_access` (instance / static member) or
`scoped_type_identifier` (type qualification). Resolution rules:

- `Foo.bar` where `Foo` is an indexed type → `bar` resolves to a
  member of `Foo` (field, method, or nested type). The `Foo` token gets
  its own `references` row (`ref_kind = "read"` for a static-member
  access, `"type_use"` if `Foo.bar` itself appears in a type position
  such as `Foo.Bar inner`).
- `a.b.c.D` (fully-qualified type) emits one `type_use` row for `D`
  with `referent_id` set to the resolved type. Intermediate package
  segments (`a`, `b`, `c`) do **not** emit rows — packages are not
  `symbol` rows in the schema.
- `this.field` — `field` resolves to the field on the enclosing class;
  `this` itself does **not** emit a row.
- `super.method()` — `method` resolves on the superclass; `super` does
  not emit a row.

## `ref_kind` decision tree

The four values are mutually exclusive and exhaustive over the
identifier occurrences this extractor emits. Decide in this order:

### `import_use`

The identifier sits anywhere inside an `import_declaration` node,
including the static variant.

- `import com.example.inventory.dto.ProductDTO;` — the final identifier
  `ProductDTO` is the import target; emit `import_use` whose
  `referent_id` is the imported symbol if indexed.
- `import static java.lang.Math.PI;` — final identifier `PI` is the
  imported static field; emit `import_use`.
- Intermediate package segments do **not** emit rows.
- Wildcard imports (`import x.y.*;`) emit zero rows — there is no single
  imported identifier.

### `type_use`

The identifier sits inside a tree-sitter node that ends up emitting (or
being part of) a `type` row per `types-java.md`. Specifically:

- Parameter type, return type, field type, local variable declared type.
- `extends` / `implements` clauses on a class or interface.
- `throws` clause exception types.
- `catch` parameter types (including each side of a `union_type`).
- Generic argument identifiers (each `type_identifier` inside
  `type_arguments`).
- Cast target (`(Foo) x` — `Foo` is `type_use`).
- `instanceof` target.
- Bounds on type parameters: `<T extends A & B>` — both `A` and `B` are
  `type_use`.
- `new Foo()` — `Foo` is `type_use` (this is the constructor target;
  the *call* edge lives in `calls`, but the identifier kind is
  `type_use`).
- Annotation target name (`@Service` — `Service` is `type_use`).
- Static-context resolution: in `Class.field`, the `Class` token's kind
  is `type_use` when `Class` is a type name; in `obj.field` (where
  `obj` is a variable), `obj`'s kind is `read`.

### `write`

The identifier is the **target** of an assignment or a mutating
operation in source. Specifically:

- Left-hand side of `=`, including the leaf of a `field_access`
  (`product.setName(…)` — note: `setName` is *not* a write, it's a
  method call which emits a `read` row; only the JLS assignment forms
  count here). For the field leaf itself, a `write` row is emitted
  **only** when the field has a known `symbol_id` in the store (per
  `docs/contract-review.md`, policy 5). Fields not extracted as
  symbols produce no field-level row.
- Left-hand side of compound assignments (`+=`, `-=`, `*=`, `/=`, `%=`,
  `&=`, `|=`, `^=`, `<<=`, `>>=`, `>>>=`) — one `write` row, no
  separate `read`. Per `docs/contract-review.md`: compound
  assignment is single-row `write` at Level 3.
- Operand of `++` / `--` (prefix or postfix) — single `write` row.
- The declared name in a `variable_declarator` *only when* the
  declarator has an initializer (`int x = 5;` — `x` is a `write`; bare
  `int x;` does not emit a row for `x` because there's no use, only a
  declaration which already lives in `symbol`).
- Initialiser-only field declarations are treated the same way.

Setter calls (`product.setName(…)`) are not writes at the JLS level —
they are method invocations. The contract therefore emits `read` for
the receiver `product` and the call itself is captured by `calls`.

### `read`

Default for any identifier that doesn't match the other three:

- Right-hand side of any expression.
- Method invocation target: in `productService.findAll(…)`,
  `productService` is `read`; the method name `findAll` is **not**
  emitted as a `references` row because the call is captured in `calls`.
- Field access leaf in a non-assignment context.
- Argument expressions.
- Return-statement value expressions.
- Annotation arguments (`@Cacheable(value = "products")` — the
  identifier `value` is **not** emitted; annotation parameter names are
  not symbols).
- `this` and `super` do **not** emit rows (they're keywords, not
  identifier symbols).
- Enum constant references (`HttpStatus.NOT_FOUND` — `HttpStatus` is
  `type_use`, `NOT_FOUND` is `read`).

### Identifiers that emit **no** row

- Package segments inside `field_access` / `scoped_type_identifier`
  (`com`, `example`, `inventory` in `com.example.inventory.X`).
- Method names at a call site (captured by `calls`).
- Constructor names in a `new` expression (the constructor target is
  emitted as `type_use` for the type; the call edge is in `calls`).
- The defining identifier of a class / method / field declaration
  itself (captured by `symbol`).
- `this`, `super`, `null`, literals.
- Labels (`break label;` / `continue label;`).

## `referent_id` resolution

The Java extractor uses a **fresh per-file scope tree**, not the global
`symbols_by_name` index from `src/graph/builder.rs`. Rationale: Java's
package + import system makes the bare name → symbol map too ambiguous
without local context. The per-file resolver caches the file's import
table and walks the AST top-down, maintaining a scope stack.

### Algorithm

For each identifier occurrence `tok` at AST node `N`:

1. Compute the enclosing scope chain by walking parent nodes upward,
   pushing a scope frame at each block / method / type / file boundary.
2. For each frame from innermost to outermost, look up `tok.text`:
   - In block frames: locals declared lexically before `tok`.
   - In method frames: parameters and method type parameters.
   - In type frames: declared members (fields, methods, nested types),
     then declared members of supertypes (one level at a time, BFS).
   - In file frames: top-level types, single-type imports, same-package
     types, on-demand imports (only if unambiguous), static-member
     imports, `java.lang` prelude.
3. On first match, emit a `references` row with that symbol's `id` as
   `referent_id`.
4. On no match across the entire chain, emit the row with
   `referent_id = null` (updated per `docs/contract-review.md`:
   the schema now allows nullable `referent_id` in value position).
   Unresolved rows let downstream audits count unresolved-rate
   and find dangling identifier sites.

### Tie-breaking

- Inner binding wins: stop walking on first hit.
- Inside a single scope frame, two members with the same simple name
  cannot coexist (Java compiler error), so no tie-breaking is required.
- Across imports: a same-package type beats an on-demand `*` import.
  An explicit single-type import beats both. Static-member imports
  apply only to non-type names and only when the file scope has no
  same-named field of the enclosing type.

### Cross-file resolution

The resolver consults the symbol table built from `symbol` rows during
the first pass of `GraphBuilder` (the existing `symbols_by_name` index
is reused for the look-ups; only the scope walk itself is
file-local). The contract does **not** require this resolution to
happen during `absorb_file_data`; it can run in a deferred pass after
the channel drains, alongside `DeferredCall` and `DeferredImport`
resolution.

### `site_file` and `site_start_byte`

- `site_file` is the file `id` (= path) where the reference occurs.
- `site_start_byte` is the tree-sitter `start_byte` of the identifier
  token itself, **not** of the enclosing expression.

The schema's primary key is `(referrer_id, site_file, site_start_byte, match_index)` (updated per `docs/contract-review.md`). Each distinct identifier occurrence in source gets its own row by virtue of the unique `site_start_byte`, so the previous "first-occurrence-wins" pair collapse no longer applies. Multiple method-call sites referring to the same callee produce multiple rows, one per site.

## Worked examples

All citations are `path:line` (1-indexed) into
`../virgil-skills/benchmarks/java/spring-api/`. `site_start_byte` is
omitted in the tables below for brevity — the contract is that it
equals the tree-sitter `start_byte` of the named identifier token.
`referrer_id` is the `symbol.id` of the enclosing function /
constructor / field initialiser (per ADR-0002:
`path|start_line|start_col|name|kind`).

### Example 1 — type uses, reads, and method-call receivers

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:43-47`

```java
public ProductDTO findById(Long id) {
    Product product = productRepository.findById(id)
            .orElseThrow(() -> new ResourceNotFoundException("Product not found: " + id));
    return toDTO(product);
}
```

Let `R = "src/.../ProductService.java|43|4|findById|method"` (the
enclosing method).

Rows emitted:

| referrer_id | referent_id (target) | ref_kind | notes |
|---|---|---|---|
| `R` | `…|ProductService.java|<line>|<col>|ProductDTO|class` (resolved via import line 3) | `type_use` | return type |
| `R` | `…|String.java|…|Long|class` (unresolved → `null` → row with `referent_id = null`) | — | `Long` is `java.lang.Long`; `java.lang` prelude does not produce indexed targets when no `java.lang.Long.java` exists in the workspace → row emitted with `referent_id = null` |
| `R` | `…|Product.java|…|Product|class` | `type_use` | local declared type |
| `R` | `…|ProductRepository.java|…|ProductRepository|class` | (no row) | `productRepository` is a *field name*, not the type. The row below covers it. |
| `R` | `…|ProductService.java|22|24|productRepository|variable` | `read` | field access |
| `R` | `R` itself (the parameter `id` symbol — but parameters don't have separate `symbol` rows under the current Java extractor, see Note) | — | see Note |
| `R` | `…|ResourceNotFoundException.java|…|ResourceNotFoundException|class` | `type_use` | `new` target |
| `R` | `…|ProductService.java|137|17|toDTO|method` | (no row — captured by `calls`) | — |
| `R` | `…|ProductService.java|43|31|product|variable` | (no row — locals not in `symbol`) | — |

**Note on parameters and locals.** The current Java extractor in
`src/languages/java/queries.rs` does not emit `parameter` or local-
variable symbols. The Level-3 implementation MUST extend the extractor
to emit `symbol` rows for parameters and locals (kind `"parameter"` /
`"variable"`) so that references to them can be resolved. Until that
lands, parameter and local references are emitted with
`referent_id = null` (per `docs/contract-review.md` policy 1; the
previous "skip" behavior was replaced by the cross-language
null-row convention).

### Example 2 — `write` to a field via setter (counter-example)

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:52-62`

```java
Product product = new Product();
product.setName(dto.getName());
product.setSku(dto.getSku());
product.setDescription(dto.getDescription());
product.setPrice(dto.getPrice());
product.setStockQuantity(dto.getStockQuantity());
product.setCategoryId(dto.getCategoryId());
product.setStatus("ACTIVE");
product.setCreatedAt(LocalDateTime.now());
product.setUpdatedAt(LocalDateTime.now());
Product saved = productRepository.save(product);
```

Common confusion: these look like writes. They are not.
`product.setName(…)` is a method call, captured by `calls`. The
identifier `product` (the receiver) is a `read`. There are no
`references` rows with `ref_kind = "write"` in this block.

The only `write` row in this block is implicit: line 52
`Product product = new Product();` — `product` is a local variable
declarator with an initializer, so `product` is a `write` referring to
its own newly-created `symbol`. (Skipped today, see Note above.)

### Example 3 — shadowing: local variable hides field

**Source.** `src/main/java/com/example/inventory/middleware/RateLimitFilter.java:30-62`

```java
protected void doFilterInternal(HttpServletRequest request,
                                HttpServletResponse response,
                                FilterChain chain)
        throws ServletException, IOException {
    String clientIp = getClientIp(request);
    ...
    RequestCount count = requestCounts.computeIfAbsent(clientIp, k -> new RequestCount());
    ...
    synchronized (count) {
        ...
        int current = count.count.incrementAndGet();
```

The class `RequestCount` (nested at line 74) has a field `count` of
type `AtomicInteger`. The method `doFilterInternal` declares a *local*
named `count` at line 43.

Inside the body, the bare identifier `count`:

- On line 43 (LHS of declarator): `write` to the new local.
- On line 46 (`synchronized (count)`): `read` of the local.
- On line 47 (`count.windowStart`): `read` of the local; the `.windowStart`
  field access resolves on the static-nested type `RequestCount`.
- On line 52 (`count.count.incrementAndGet()`): the **outer** `count`
  is the local (read); the **inner** `.count` is a field on
  `RequestCount` (read of the nested-class field).

There is no `RateLimitFilter` field named `count`, so this example
demonstrates *nested-type* shadowing rather than class-field shadowing.

For a class-field shadowing case, the contract specifies: if a method
body declared `private int count;` at class scope and a local
`int count = 0;` inside, every bare `count` inside the local's scope
resolves to the local; only `this.count` resolves to the field.

### Example 4 — static-member access (`HttpStatus.PAYMENT_REQUIRED`)

**Source.** `src/main/java/com/example/inventory/exception/PaymentException.java:10`

```java
@ResponseStatus(HttpStatus.PAYMENT_REQUIRED)
public class PaymentException extends RuntimeException {
```

Rows emitted in the file-level scope (the annotation precedes the
class declaration; for `referrer_id` purposes the annotation is
treated as belonging to the class symbol — line 11):

Let `R = "…/PaymentException.java|11|13|PaymentException|class"`.

| referrer_id | referent | ref_kind | notes |
|---|---|---|---|
| `R` | (org.springframework.web.bind.annotation.ResponseStatus — not indexed) | — | row with `referent_id = null` |
| `R` | (org.springframework.http.HttpStatus — not indexed) | — | row with `referent_id = null` |
| `R` | `RuntimeException` (java.lang prelude, not indexed as source) | — | row with `referent_id = null` |

When the Spring source jars *are* indexed, the rows are:

| referrer_id | referent_id | ref_kind |
|---|---|---|
| `R` | `…/ResponseStatus.java|…|ResponseStatus|class` | `type_use` |
| `R` | `…/HttpStatus.java|…|HttpStatus|class` | `type_use` |
| `R` | `…/HttpStatus.java|…|PAYMENT_REQUIRED|variable` | `read` |
| `R` | `…/RuntimeException.java|…|RuntimeException|class` | `type_use` |

Key resolver decision: in `HttpStatus.PAYMENT_REQUIRED`, the parser
sees a `field_access` with object `HttpStatus` and field
`PAYMENT_REQUIRED`. The resolver checks whether the object position is
a type name (via the import table + scope) — it is — and so emits
`type_use` for `HttpStatus` and `read` for `PAYMENT_REQUIRED`.

### Example 5 — import_use rows

**Source.** `src/main/java/com/example/inventory/service/ProductService.java:1-17`

```java
package com.example.inventory.service;

import com.example.inventory.dto.ProductDTO;
import com.example.inventory.dto.UpdateProductRequest;
import com.example.inventory.model.Product;
import com.example.inventory.repository.ProductRepository;
import com.example.inventory.exception.ResourceNotFoundException;
import com.example.inventory.exception.ValidationException;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.cache.annotation.CacheEvict;
import org.springframework.cache.annotation.Cacheable;
import org.springframework.stereotype.Service;

import java.math.BigDecimal;
import java.time.LocalDateTime;
import java.util.List;
import java.util.stream.Collectors;
```

Each `import_declaration` emits one `import_use` row, *referrer_id =
the file-level pseudo-symbol* (use the package-declaration's symbol
when present; otherwise the file's `id`).

Indexed-target rows (sources present in the benchmark):

| referent_id | ref_kind |
|---|---|
| `…/ProductDTO.java|…|ProductDTO|class` | `import_use` |
| `…/UpdateProductRequest.java|…|UpdateProductRequest|class` | `import_use` |
| `…/Product.java|…|Product|class` | `import_use` |
| `…/ProductRepository.java|…|ProductRepository|class` | `import_use` |
| `…/ResourceNotFoundException.java|…|ResourceNotFoundException|class` | `import_use` |
| `…/ValidationException.java|…|ValidationException|class` | `import_use` |

The Spring and `java.*` imports do not have indexed sources in the
spring-api benchmark, so they're skipped (no row emitted) — they still
appear in the existing `raw_import` / `imports` rows, which capture
unresolved imports.

### Example 6 — write via assignment (counter-example with `+=`)

**Source.** `src/main/java/com/example/inventory/middleware/RateLimitFilter.java:43-52`

```java
RequestCount count = requestCounts.computeIfAbsent(clientIp, k -> new RequestCount());

long now = System.currentTimeMillis();
synchronized (count) {
    if (now - count.windowStart > WINDOW_MILLIS) {
        count.windowStart = now;
        count.count.set(0);
    }

    int current = count.count.incrementAndGet();
```

Line 48: `count.windowStart = now;` — this is a `field_access` on the
LHS of an `=`. Rows emitted:

| referrer_id | referent_id | ref_kind | notes |
|---|---|---|---|
| `R = "…/RateLimitFilter.java|30|14|doFilterInternal|method"` | `…|RateLimitFilter.java|43|22|count|variable` (local — currently skipped, see Note in Example 1) | `read` | `count` is the LHS object |
| `R` | `…|RateLimitFilter.java|76|13|windowStart|variable` (nested-class field) | `write` | the leaf of the field_access on the assignment LHS |
| `R` | `…|RateLimitFilter.java|45|13|now|variable` (local) | `read` | RHS |

Line 49: `count.count.set(0);` — this is a method call (`set`), not an
assignment. No `write` row is emitted. The two `count` tokens are both
`read` (one is the local, one is the field).

Line 52: `int current = count.count.incrementAndGet();` —
`current` is a `write` of a new local; the rest is reads, and the
method call is captured by `calls`.

### Example 7 — multi-catch type_use (forward-looking)

The benchmark uses sequential `catch` clauses, not multi-catch.
Contract: a hypothetical
`catch (SignatureException | RuntimeException e)` emits:

- One `type` row of kind `"union"` (per `types-java.md` Example 6).
- One `references` row for `SignatureException` (`type_use`, target =
  the imported class).
- One `references` row for `RuntimeException` (`type_use`, target =
  `java.lang.RuntimeException` if indexed, else skipped).
- No row for the variable name `e` (parameter declaration, handled by
  `symbol`).
- No row for the `|` token.
