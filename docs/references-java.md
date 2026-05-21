# References — Java

Per [ADR-0005](adr/0005-datalog-resolution.md), this contract describes
**fact emission** only. The Java extractor emits `occurrence`, `scope`,
and `binding` rows. Resolution (turning each `occurrence` into a
`referent_id` in `references`) lives in `docs/resolution.md` as
Cozoscript rules that apply uniformly across all languages. No
`references` rows are produced by the extractor.

Symbol IDs in worked examples follow
[ADR-0002](adr/0002-symbol-id-scheme.md):
`path|start_line|start_col|name|kind`. `start_byte` values are the
tree-sitter `Range.start_byte` of the relevant node (no trivia
adjustment).

> **Prerequisite — review item 7.** The Java extractor in
> `src/languages/java/queries.rs` does not currently emit `parameter` or
> local-variable symbol rows. This contract assumes
> [Issue #11](../../../) (parameter/local extraction) lands **before**
> [Issue #16](../../../). The `binding` rows of `binding_kind =
> "parameter"` produced here reference those symbol rows by id. Until
> Issue #11 lands, the parameter bindings still get emitted with their
> ADR-0002 ids, but the matching `symbol` rows won't exist and the
> resolver will treat references to them as unresolved.

## Scope tree

Java's lexical scopes that the extractor emits as `scope` rows:

| Java construct | tree-sitter node | `scope.kind` | Notes |
|---|---|---|---|
| Compilation unit (file) | the `program` root | `"file"` | One per file; `parent_id = null`. The package declaration's name is not a separate scope — package membership is recorded via `imports` and via top-level type visibility, not via a `scope` row. |
| Package (compile-unit grouping) | not directly a node | `"module"` | Java has **one** package per file; the file scope's `kind` is `"file"`. The contract emits **no** separate `"module"` scope row for the package — package-level resolution is done by the resolver using `imports` plus file-scope bindings of indexed same-package types. |
| Top-level class / interface / enum / record / annotation type | `class_declaration`, `interface_declaration`, `enum_declaration`, `record_declaration`, `annotation_type_declaration` | `"class"` | One public class per `.java` file is the JLS rule; the extractor emits one `"class"` scope per type body regardless. |
| Inner class / nested class / local class | `class_declaration` etc. inside another type or block | `"class"` | Each inner class gets its own class scope; `parent_id` is the enclosing class or block. |
| Anonymous class | `object_creation_expression` whose `class_body` child is present | `"class"` | The `class_body` opens a new class scope whose `parent_id` is the enclosing block. |
| Method / constructor | `method_declaration`, `constructor_declaration` | `"function"` | Parameters and the body share the same `"function"` scope. Type parameters `<T>` are also bound here. |
| Static initializer | `static_initializer` | `"function"` | Treated as its own function-like scope so that locals declared in it don't leak to peer initializers. |
| Instance initializer | `block` directly in a class body | `"function"` | Same treatment as static initializers. |
| Lambda | `lambda_expression` | `"function"` | The lambda parameters and the lambda body share one `"function"` scope, parented to the enclosing block. Single-expression lambdas (no braces) still get a scope row. |
| Block | every `{ ... }` (`block`, `constructor_body`, `enhanced_for_statement` body, `catch_clause` body, etc.) | `"block"` | One per syntactic block, parented to the innermost enclosing scope. |
| `for`-header | `for_statement` parenthesised header | `"block"` | A separate block scope wrapping the body so that loop-declared variables (`for (int i = 0; …)`) shadow names in the enclosing block but not in sibling statements. |
| `try`-with-resources header | `try_with_resources_statement` resource list | `"block"` | Wraps the `try` body so the resource variable is visible only inside the `try`/`catch`/`finally`. |
| `catch` parameter | `catch_clause` | `"block"` | The catch clause opens a block scope holding the catch parameter binding(s). |

`parent_id` of each emitted scope is the innermost enclosing scope.
`scope.id = file_path|start_byte|kind` per the schema.

### What does NOT open its own scope

- `if` / `else` branches without explicit braces — only the wrapped
  block opens a scope; a single-statement `if x;` does not.
- Switch statement bodies — the `switch_block` is a single `"block"`
  scope; individual `case` labels do **not** open sub-scopes.
- The annotation list above a declaration — annotations live in the
  enclosing scope; they don't introduce a scope of their own.

## Bindings

For each `binding_kind`, the AST patterns the extractor recognises.
`binding.id` is keyed by `(scope_id, name, start_byte)`; `symbol_id` is
the target `symbol.id` when indexed, else `null`.

### `definition`

A site that introduces a name in its enclosing scope. The `symbol_id`
is the same id the `symbol` extractor emits for the definition.

| Java construct | Bound name | Bound in scope |
|---|---|---|
| `class_declaration` / `interface_declaration` / `enum_declaration` / `record_declaration` / `annotation_type_declaration` | type's simple name | enclosing file or class scope (top-level types live in the file scope; nested types live in the class scope of the enclosing type) |
| `method_declaration` | method name | enclosing class scope |
| `constructor_declaration` | the enclosing class's name | enclosing class scope (note: Java permits multiple constructors with the same name; the resolver disambiguates via `match_index` per [ADR-0003](adr/0003-level-3-types-and-references.md)) |
| `field_declaration` (each `variable_declarator` inside it) | field name | enclosing class scope |
| `enum_constant` | constant name | enclosing class (enum) scope |
| `record_component` | component name | enclosing class scope and as a parameter for the canonical constructor (one `definition` row plus one `parameter` row) |
| `static_initializer` | no `definition` row (no introduced name) | — |

### `parameter`

Method, constructor, and lambda parameter declarations. Emitted in the
enclosing function scope.

| Source | `name` | `scope_id` |
|---|---|---|
| `formal_parameter` inside a `method_declaration` | parameter name | the method's `"function"` scope |
| `formal_parameter` inside a `constructor_declaration` | parameter name | the constructor's `"function"` scope |
| `inferred_parameters` and `formal_parameters` inside a `lambda_expression` | each parameter name | the lambda's `"function"` scope |
| `catch_formal_parameter` inside a `catch_clause` | exception variable name | the catch clause's `"block"` scope |
| `resource` inside a `try_with_resources_statement` (declared variable) | resource variable name | the try-resources `"block"` scope |
| `local_variable_declaration` | declared variable name | enclosing `"block"` scope (a *local* binding, but encoded as `binding_kind = "parameter"` per ADR-0005 because parameter and local both denote a function-scoped or block-scoped name resolved via the `symbol` row — see [Issue #11](../../../)) |
| `enhanced_for_statement` declared variable (`for (Foo x : items)`) | loop variable | the `for`-header `"block"` scope |

> The `binding_kind` enum in the schema (`docs/virgil-datalog-schema.md`)
> spans `definition`, `parameter`, `import`, `import_alias`,
> `wildcard_import`. Locals are emitted as `parameter` (the schema's
> name for "scope-resolved, non-import binding rooted in a `symbol`
> row"). If a future schema introduces a distinct `local` kind, this
> contract gets updated; until then, all in-function bindings to symbol
> rows go through `parameter`.

`symbol_id` matches the `symbol` row produced by the parameter/local
extractor in Issue #11.

### `import`

Plain single-type imports. Bound in the file scope.

- `import_declaration` of the form `import a.b.C;` — emits one
  `import` binding with `name = "C"`, `scope_id` = the file's
  `"file"` scope, `symbol_id` = the imported type's `symbol.id` if the
  target file is indexed, else `null`.
- `import_declaration static` of the form
  `import static a.b.X.METHOD;` — emits one `import` binding with
  `name = "METHOD"`, `scope_id` = file scope, `symbol_id` pointing at
  the static member (method or field) if indexed. **Static imports are
  emitted as plain `import` bindings, not as `import_alias`** — Java
  has no syntactic alias; the bound name equals the trailing
  identifier of the import path.

### `import_alias`

**Java has no `import as` syntax.** The `import_alias` kind is therefore
**never** emitted by the Java extractor. This is an explicit
non-finding: if a Java-aware audit needs to know "all aliased
imports", the answer over a Java workspace is always the empty set.

### `wildcard_import`

On-demand imports. One row per wildcard declaration with `name = "*"`,
`symbol_id = null` (the resolver expands at materialise time using the
`imports` graph).

- `import a.b.*;` → `binding{scope_id: <file>, name: "*", binding_kind: "wildcard_import", symbol_id: null}`.
- `import static a.b.X.*;` → same shape. The resolver distinguishes
  type-on-demand from static-on-demand by inspecting the imported
  target's nature (file-of-types vs class-of-static-members); the
  extractor does not pre-classify.

## Occurrence emission

For each `occurrence_kind`, the AST patterns the extractor emits.
`occurrence.id = path|start_byte|name|occurrence_kind`.
`enclosing_symbol_id` is the innermost containing symbol (`null` for
expressions outside any symbol — e.g. annotation arguments on the
top-level type). `enclosing_scope_id` is the innermost containing
`scope` row.

### `call`

Every call expression's callee identifier:

- `method_invocation` — the `name` field (the leaf identifier in
  `foo()` or in `obj.foo()`). For `obj.foo()`, only `foo` is emitted as
  `call`; `obj` is emitted separately as a `read` (see below).
- `object_creation_expression` (`new Foo(...)`) — emits a `type_use`
  for `Foo` (constructor target is a type-position identifier) and
  emits **no** `call` occurrence. The call edge into the constructor is
  materialised by the resolver from the `type_use` occurrence joined
  with constructor `definition` bindings, or stays in the existing
  `calls` relation if Phase 1 populates it directly.
- `explicit_constructor_invocation` (`this(...)` or `super(...)`) —
  emits a `call` occurrence with `name = "this"` or `"super"`. The
  resolver treats `this`/`super` constructor calls as resolved against
  the enclosing class / superclass via dedicated bindings (not in
  scope of this contract).

### `read`

Every identifier in value position. Specifically:

- Variable / field / parameter / local references on the RHS of any
  expression.
- The receiver in a method invocation (`obj` in `obj.foo()`).
- The qualifier in a static-member access (`HttpStatus` in
  `HttpStatus.PAYMENT_REQUIRED` — emitted as `read` of the type name;
  the resolver determines whether the target is a type by joining with
  `definition` bindings).
- Enum constant references (`HttpStatus.PAYMENT_REQUIRED` →
  `read` for `PAYMENT_REQUIRED`).
- Arguments to method invocations, constructor invocations,
  annotations.
- Return-statement values.
- Expressions inside `if` / `while` / `for` / `switch` headers.

**`this` field-access rule.** For `this.x`:

- Emit a `read` occurrence with `name = "this"`. The resolver binds
  `this` to the enclosing class (via an implicit class-scope
  `"parameter"` binding established when the enclosing scope's kind is
  `"function"` inside a class — extractor responsibility, but it falls
  out of the parameter-emission pass in Issue #11).
- Do **not** emit any occurrence for the field name `x` after the
  dot. The resolver resolves field accesses by joining the `read of
  this` occurrence with `definition` bindings of kind `field` in the
  enclosing class scope. This avoids duplicating the field access as
  both an occurrence and a `references` row for a name that's only
  meaningful in the context of its receiver.

This is the cross-language rule from `docs/resolution.md`: the field
access's name resolution is a join over bindings, not an occurrence
emission. The same rule applies to `super.x` (emit `read` of `super`,
omit `x`).

**Identifiers that emit no `read` row** (suppressions):

- The method name at a call site (covered by `call`, above).
- Package segments in a fully-qualified name (`com`, `example`,
  `inventory` in `com.example.inventory.X` inside an
  `import_declaration` or `scoped_type_identifier`). Packages are not
  `symbol` rows.
- The defining identifier of a class / method / field declaration
  (covered by `symbol` and by the matching `definition` binding).
- `null`, literals, labels in `break label;` / `continue label;`.
- Annotation parameter names (`@Cacheable(value = "products")` — the
  bare `value` is **not** an occurrence; it names an annotation element
  in the annotation type, not a local symbol).

### `write`

The identifier is the target of an assignment or a mutating operation.
Per [ADR-0003](adr/0003-level-3-types-and-references.md), compound
assignments emit a **single** `write` occurrence (no separate `read`).

- LHS of `=`, when the LHS is a bare identifier. Emit `write` with
  `name = <identifier>`.
- LHS of `=` when the LHS is a `field_access` (`obj.field = x`,
  `this.field = x`): emit `read` for the receiver (`obj` or `this`)
  and `write` for the field leaf. The field leaf is emitted as a
  `write` occurrence on the field's simple name — the resolver matches
  it against `definition` bindings of `field` kind in the receiver's
  type. (Per `docs/contract-review.md` policy 5, a downstream
  consumer can filter out field-leaf writes whose target field has no
  `symbol` row.)
- LHS of any compound assignment (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`,
  `|=`, `^=`, `<<=`, `>>=`, `>>>=`) — single `write`, no `read`.
- Operand of `++` / `--` (prefix or postfix) — single `write`.
- A `variable_declarator` with an initializer — the declared name
  emits a `write` occurrence (paired with the `definition` /
  `parameter` binding for the same name; the resolver then renders
  this as a self-reference to the freshly-declared symbol).
- Field declarations with initializers (`private int x = 5;`) emit a
  `write` occurrence on `x`.

### `type_use`

The identifier sits in a type position. These overlap exactly with the
`type` rows produced by `types-java.md`. Specifically:

- Parameter type, return type, field type, local variable declared
  type, record component type.
- `extends` clause target on `class_declaration` /
  `interface_declaration`. `implements` clause targets.
- `throws` clause exception types on methods and constructors.
- `catch_formal_parameter` type(s) — each branch of a `union_type`
  is a separate `type_use`.
- Generic argument identifiers inside `type_arguments`
  (each `type_identifier`).
- Cast target (`(Foo) x` — `Foo` is `type_use`).
- `instanceof` target (`x instanceof Foo`).
- Type-parameter bounds (`<T extends A & B>` — both `A` and `B` are
  `type_use`).
- The class name in `new Foo(...)` (object creation).
- The class name in array creation (`new Foo[10]` — `Foo` is
  `type_use`).
- The class name in `.class` literal (`Foo.class` — `Foo` is
  `type_use`).
- The annotation name in **any** `@Annotation` usage. This includes:
  type annotations (`@Service`, `@RestController`), method/field
  annotations (`@Autowired`, `@Override`), parameter annotations
  (`@RequestBody`), and annotation arguments that are themselves
  annotations. The annotation's simple name (e.g. `Service`) is
  emitted as a `type_use` occurrence with `enclosing_symbol_id` set to
  the annotated declaration.

### `import_use`

The identifier sits inside an `import_declaration`. Emit one
`import_use` occurrence for the **final** identifier of the import
path (the imported simple name) and zero rows for intermediate
package segments.

- `import a.b.C;` — one `import_use` on `C`.
- `import static a.b.X.METHOD;` — one `import_use` on `METHOD`.
- `import a.b.*;` — zero `import_use` rows (there is no single
  imported identifier; the binding row of kind `wildcard_import` is
  the only fact emitted).
- `import static a.b.X.*;` — same: zero `import_use` rows; the
  `wildcard_import` binding carries everything the resolver needs.

`enclosing_symbol_id` for `import_use` is `null` (imports sit at file
scope, outside any symbol). `enclosing_scope_id` is the file scope.

## What this contract does NOT cover

- **Resolution algorithm.** Lives in `docs/resolution.md`, applied
  uniformly. Walking scopes outward, picking the innermost binding,
  expanding wildcards, and disambiguating overloads are all the
  resolver's job. This contract describes only the inputs.
- **Method dispatch / overload selection.** Java permits multiple
  methods with the same name in the same class scope. The extractor
  emits one `definition` binding per overload at the class scope.
  The resolver materialises multiple `references` rows with
  `match_index = 0, 1, 2, …` for the same call site, one per
  candidate. Selecting *which* overload actually runs is dynamic and
  out of scope; the resolver's job is to enumerate the candidates.
- **Reflection-based dispatch.** `Class.forName("Foo")`,
  `method.invoke(...)`, etc., are method invocations like any other
  — they generate `call` occurrences whose argument strings happen to
  name types. Treating those strings as references is out of scope.
- **`references` rows.** Worked examples below show the *inputs* to
  resolution (occurrences, scopes, bindings), not the resolver's
  output rows.

## Worked examples

Seven examples drawn from `../virgil-skills/benchmarks/java/spring-api/`.
For brevity, columns shown are the schema-relevant ones; `start_byte`
values are summarised as `bN` (the tree-sitter `start_byte` of the
named identifier or block opener — substitute the actual integer when
emitting rows). `path` is abbreviated to the file's basename.

For each example, the rows emitted are:

1. `scope` rows for the example's range.
2. `binding` rows that fall inside that range.
3. `occurrence` rows for every identifier the contract specifies.

### Example 1 — `@Service` annotation usage on a class declaration

**Source.** `service/ProductService.java:19-20`

```java
@Service
public class ProductService {
```

**Scopes (new, opened by this snippet):**

| id | parent | kind | start | notes |
|---|---|---|---|---|
| `ProductService.java\|b_file\|file` | `null` | `file` | byte 0 | the file's root scope (opened earlier in the file at byte 0; included here for context) |
| `ProductService.java\|b_class\|class` | `ProductService.java\|b_file\|file` | `class` | byte at `{` after `ProductService` | class body scope |

**Bindings (introduced by this snippet):**

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| file scope | `Service` | start of `Service` in `import org.springframework.stereotype.Service;` (line 12) | `null` (Spring not indexed) | `import` |
| file scope | `ProductService` | byte of `ProductService` at line 20 col 14 | `ProductService.java\|20\|14\|ProductService\|class` | `definition` |

**Occurrences:**

| id | name | kind | enclosing_symbol_id | enclosing_scope_id |
|---|---|---|---|---|
| `ProductService.java\|<b of Service at line 19>\|Service\|type_use` | `Service` | `type_use` | `ProductService.java\|20\|14\|ProductService\|class` | file scope |

Key decisions:

- The annotation `@Service` emits **one** `type_use` occurrence on the
  annotation's simple name. The resolver looks `Service` up via the
  file scope's `import` binding for `Service` and finds the import row
  (its `symbol_id` is `null` because Spring isn't indexed, so the
  resolver emits a `references` row with `referent_id = null`).
- The `@` token and parenthesised arguments (none here) emit no
  occurrences.
- `enclosing_symbol_id` for the annotation occurrence is the class
  symbol itself — the annotation is attached to the class declaration.

### Example 2 — `@Autowired` field injection (annotation + field-type type_use)

**Source.** `service/ProductService.java:22-23`

```java
    @Autowired
    private ProductRepository productRepository;
```

**Bindings (introduced by this snippet):**

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| class scope | `productRepository` | byte at line 23 col 31 | `ProductService.java\|23\|31\|productRepository\|field` | `definition` |

(The `Autowired` and `ProductRepository` `import` bindings are at the
file scope, established earlier — line 4 and line 9.)

**Occurrences:**

| id | name | kind | enclosing_symbol_id | enclosing_scope_id |
|---|---|---|---|---|
| `…\|<b of Autowired>\|Autowired\|type_use` | `Autowired` | `type_use` | the field symbol `…\|23\|31\|productRepository\|field` | class scope |
| `…\|<b of ProductRepository>\|ProductRepository\|type_use` | `ProductRepository` | `type_use` | the field symbol | class scope |

Notes:

- The field initializer is absent, so `productRepository` emits **no**
  `write` occurrence on this line (only the `definition` binding for it,
  shown above).
- Annotation simple names always emit `type_use`. Field types always
  emit `type_use`. Both rows share the same `enclosing_symbol_id` (the
  field) because both attach to the field declaration.

### Example 3 — Static import absence (counter-example)

**Source.** `service/ProductService.java:1-17` (all imports)

```java
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

**Bindings (all in file scope):**

| name | symbol_id | kind |
|---|---|---|
| `ProductDTO` | `ProductDTO.java\|…\|class` | `import` |
| `UpdateProductRequest` | `UpdateProductRequest.java\|…\|class` | `import` |
| `Product` | `Product.java\|…\|class` | `import` |
| `ProductRepository` | `ProductRepository.java\|…\|class` | `import` |
| `ResourceNotFoundException` | `ResourceNotFoundException.java\|…\|class` | `import` |
| `ValidationException` | `ValidationException.java\|…\|class` | `import` |
| `Autowired` | `null` | `import` |
| `CacheEvict` | `null` | `import` |
| `Cacheable` | `null` | `import` |
| `Service` | `null` | `import` |
| `BigDecimal` | `null` | `import` |
| `LocalDateTime` | `null` | `import` |
| `List` | `null` | `import` |
| `Collectors` | `null` | `import` |

**Occurrences:**

One `import_use` per import line on the trailing simple name. No
`import_alias` rows are produced. No static imports appear anywhere in
this file (or in the `spring-api` benchmark — verified by grep for
`import static` — so the **absence** of `import_alias` rows is the
finding: a Java audit asking for "list of all aliased imports in this
codebase" returns the empty set, and not because the extractor missed
anything but because Java doesn't support the feature.

### Example 4 — Shadowing: method parameter shadows a class field

**Source.** `model/Product.java:48-49` — the `setName` setter

```java
    public String getName() { return name; }
    public void setName(String name) { this.name = name; }
```

This is the canonical Java shadowing case: a setter whose parameter has
the same simple name as the field it assigns.

**Scopes (new):**

| id | parent | kind | start | notes |
|---|---|---|---|---|
| `Product.java\|b_setName\|function` | class scope | `function` | byte at `setName` parameter list | method scope holds the parameter binding |

**Bindings:**

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| class scope (introduced earlier on line 16) | `name` | byte at line 16 col 19 | `Product.java\|16\|19\|name\|field` | `definition` |
| setName function scope | `name` | byte at line 49 col 33 (the parameter) | `Product.java\|49\|33\|name\|parameter` | `parameter` |

**Occurrences inside `setName`'s body** (`{ this.name = name; }`):

| name | kind | enclosing_symbol_id | enclosing_scope_id | notes |
|---|---|---|---|---|
| `this` | `read` | `Product.java\|49\|17\|setName\|method` | setName function scope | the receiver of the assignment |
| `name` (the field leaf of `this.name`) | `write` | setName method | setName function scope | LHS of `=`; field is resolved by joining with the field's `definition` binding in `Product`'s class scope |
| `name` (the RHS) | `read` | setName method | setName function scope | bare `name` resolves to the **parameter** (innermost binding wins); the field is shadowed |

Key decisions:

- The receiver `this` and the field leaf of `this.name` together cover
  the field write: `this` emits a `read` occurrence; the field leaf
  emits a `write`. The resolver does the join.
- The RHS bare `name` resolves to the parameter binding in
  `setName`'s function scope, **not** to the field. This is the
  shadowing rule from `docs/resolution.md` falling out of
  innermost-binding-wins.
- No occurrence is emitted for the method name `setName` (covered by
  `symbol` plus `definition` binding).
- No occurrence for the parameter type `String` is shown here; it's an
  ordinary `type_use` on the parameter's type annotation.

### Example 5 — `this.field = field` write+read pair (canonical constructor / DTO pattern)

**Source.** `exception/PaymentException.java:22-26`

```java
    public PaymentException(String message, String errorCode) {
        super(message);
        this.transactionId = null;
        this.errorCode = errorCode;
    }
```

**Bindings (new in this snippet):**

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| ctor function scope | `message` | byte of `message` param | `PaymentException.java\|22\|41\|message\|parameter` | `parameter` |
| ctor function scope | `errorCode` | byte of `errorCode` param | `PaymentException.java\|22\|57\|errorCode\|parameter` | `parameter` |

**Occurrences (constructor body lines 23-25):**

| name | kind | enclosing_symbol_id | notes |
|---|---|---|---|
| `super` | `call` | the constructor symbol | explicit constructor invocation |
| `message` | `read` | the constructor symbol | argument to `super(...)` |
| `this` (line 24) | `read` | the constructor symbol | receiver of the first assignment |
| `transactionId` | `write` | the constructor symbol | field leaf on LHS — resolver joins with field `definition` in `PaymentException`'s class scope |
| `this` (line 25) | `read` | the constructor symbol | receiver of the second assignment |
| `errorCode` (line 25, LHS field leaf) | `write` | the constructor symbol | field leaf on LHS |
| `errorCode` (line 25, RHS) | `read` | the constructor symbol | resolves to the parameter (shadowing again — but this time, **because of the `this.` prefix on the LHS**, both the field write and the parameter read are unambiguous and distinct) |

Key decisions:

- Per the `this` field-access rule: the LHS `this.errorCode` produces
  one `read` occurrence on `this` plus one `write` occurrence on
  `errorCode` (the field). The RHS bare `errorCode` is a separate
  occurrence — a `read` of the parameter.
- `null` on line 24 emits no occurrence (literal).
- The `super(...)` call gets `name = "super"`, kind `call`. The
  argument `message` is a `read`.

### Example 6 — Compound assignment + nested-class field access

**Source.** `middleware/RateLimitFilter.java:46-52`

```java
        synchronized (count) {
            if (now - count.windowStart > WINDOW_MILLIS) {
                count.windowStart = now;
                count.count.set(0);
            }

            int current = count.count.incrementAndGet();
```

**Scopes (new in this snippet):**

| id | parent | kind | notes |
|---|---|---|---|
| outer `block` for `synchronized` body | `doFilterInternal` function scope | `block` | opened at byte of `{` after `synchronized (count)` |
| inner `block` for `if`-body | outer block | `block` | opened at byte of `{` after the `if` condition |

(Note: the `synchronized` keyword does not by itself introduce a
scope; only the wrapped block does.)

**Bindings (new):**

| scope_id | name | symbol_id | kind |
|---|---|---|---|
| outer `synchronized` block | none introduced here (the `count` local was bound earlier at line 43) | — | — |
| inner `if` block | none introduced (no declarations) | — | — |
| line 52 — the `for`-style declaration is actually inside `doFilterInternal`'s outer scope after the synchronized block exits | `current` (local) | `RateLimitFilter.java\|52\|17\|current\|parameter` | `parameter` |

**Occurrences (lines 46-52):**

| line | name | kind | enclosing_symbol_id | notes |
|---|---|---|---|---|
| 46 | `count` | `read` | doFilterInternal method | argument to `synchronized` — bare identifier |
| 47 | `now` | `read` | method | bare local |
| 47 | `count` | `read` | method | receiver of `count.windowStart` |
| 47 | `windowStart` | `read` | method | field leaf of `count.windowStart` (read context: inside a comparison) |
| 47 | `WINDOW_MILLIS` | `read` | method | static field of enclosing class |
| 48 | `count` | `read` | method | receiver of LHS |
| 48 | `windowStart` | `write` | method | LHS field leaf |
| 48 | `now` | `read` | method | RHS |
| 49 | `count` | `read` | method | outer receiver |
| 49 | `count` | `read` | method | inner receiver (field on `RequestCount`) |
| 49 | `set` | `call` | method | method invocation leaf |
| 52 | `current` | `write` | method | declarator with initialiser |
| 52 | `count` | `read` | method | outer receiver |
| 52 | `count` | `read` | method | inner receiver |
| 52 | `incrementAndGet` | `call` | method | method invocation leaf |

Key decisions:

- No `write` row is emitted for `count.count.set(0)` (line 49) —
  `set(...)` is a method call, not an assignment. The arg `0` is a
  literal (no occurrence). The two `count` tokens are both `read`
  occurrences with the same `name`; the resolver disambiguates by
  scope: the outer `count` resolves to the local at line 43; the
  inner `count` resolves to the `RequestCount.count` field via the
  resolver's join over field-leaf occurrences.
- Line 52 declarator: `int current = count.count.incrementAndGet();`
  emits **one** `write` for `current` (compound: declaration + init in
  a single `variable_declarator`) and `read`s for both `count`s, then
  a `call` for `incrementAndGet`. Per ADR-0003 the declarator with an
  initializer is **one** write row; there's no separate `read` of
  `current`.
- A hypothetical compound `count += 1` would emit a **single** `write`
  on `count`, not a paired read+write. The benchmark doesn't contain
  one in this region; this is a contract statement, not a worked row.

### Example 7 — Wildcard import + lambda parameter scope

**Source.** `middleware/AuthFilter.java:1-46` (relevant excerpt)

```java
import io.jsonwebtoken.Claims;
import io.jsonwebtoken.Jwts;
import io.jsonwebtoken.SignatureException;
import org.springframework.web.filter.OncePerRequestFilter;

import javax.servlet.FilterChain;
...
import java.util.Arrays;
import java.util.List;

public class AuthFilter extends OncePerRequestFilter {
    ...
    protected void doFilterInternal(HttpServletRequest request, ...
            throws ServletException, IOException {
        String path = request.getRequestURI();
        ...
        boolean isPublic = PUBLIC_PATHS.stream().anyMatch(path::startsWith);
```

This file has no wildcard imports, but `controller/SearchController.java`
does. We use the lambda example here and reference the wildcard case
from `SearchController`.

**Wildcard binding (from `controller/AuthController.java:10`):**

```java
import org.springframework.web.bind.annotation.*;
```

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| `AuthController.java` file scope | `*` | byte of the import statement's start | `null` | `wildcard_import` |

Zero `import_use` occurrences for this line.

**Lambda scope (a synthetic example mirroring line 42 of `AuthFilter`):**

The expression `PUBLIC_PATHS.stream().anyMatch(path::startsWith)` is
**not** a lambda — it's a method reference. A real lambda inside the
benchmark, `RateLimitFilter.java:43`:

```java
RequestCount count = requestCounts.computeIfAbsent(clientIp, k -> new RequestCount());
```

The lambda `k -> new RequestCount()`:

**Scopes (new):**

| id | parent | kind | notes |
|---|---|---|---|
| lambda scope | enclosing block (line 43 is inside `doFilterInternal`'s body block, prior to the synchronized block) | `function` | one `"function"` scope holding the lambda parameter and the lambda body |

**Bindings:**

| scope_id | name | start_byte | symbol_id | kind |
|---|---|---|---|---|
| lambda scope | `k` | byte of `k` | `RateLimitFilter.java\|43\|68\|k\|parameter` | `parameter` |

**Occurrences inside the lambda body** (`new RequestCount()`):

| name | kind | enclosing_symbol_id | enclosing_scope_id |
|---|---|---|---|
| `RequestCount` | `type_use` | the enclosing method symbol (lambdas are anonymous; the resolver uses the enclosing named symbol for `enclosing_symbol_id`) | lambda scope |

Key decisions:

- Lambdas are anonymous; they have no `symbol` row, so
  `enclosing_symbol_id` for occurrences inside a lambda is the
  enclosing named symbol (the method `doFilterInternal`).
- The lambda parameter `k` opens a **function** scope, not a block
  scope — parameter shadowing relative to the enclosing block is
  resolved by the resolver via scope-kind precedence in
  `docs/resolution.md`.
- A wildcard import like
  `import org.springframework.web.bind.annotation.*;` produces **one**
  `binding` row with `name = "*"`, `binding_kind = "wildcard_import"`,
  `symbol_id = null`, and **zero** occurrence rows. The resolver
  expands the wildcard at materialise time using the `imports` graph.

---

**Done-criterion.** The Java extractor is finished when, fed the
`spring-api/` benchmark and Issue #11's parameter/local symbol
extraction, it produces exactly the `scope` / `binding` / `occurrence`
rows enumerated in Examples 1–7 above. The `references` rows that
fall out of resolving those facts via `docs/resolution.md` are the
responsibility of the resolver test suite, not this contract.
