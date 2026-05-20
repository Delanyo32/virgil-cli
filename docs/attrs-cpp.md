# Language attributes — C++

This document is the contract for the `cpp_attrs` relation declared in [`virgil-datalog-schema.md`](virgil-datalog-schema.md). It documents which AST constructs populate each column and the default behavior when the construct is absent. `symbol_id` strings follow [ADR-0002](adr/0002-symbol-id-scheme.md): `path|start_line|start_col|name|kind`.

Scope: C++ source files (`.cpp`, `.cc`, `.cxx`) and C++ headers (`.hpp`, `.hxx`, `.hh`). `.h` maps to C and is governed by `c_attrs`, not `cpp_attrs`.

---

## Schema

```
:create cpp_attrs {
    symbol_id: String =>
    is_virtual:    Bool default false,
    is_const:      Bool default false,
    is_noexcept:   Bool default false,
    is_template:   Bool default false,
    is_constexpr:  Bool default false,
    is_override:   Bool default false,
    is_final:      Bool default false,
}
```

The four columns `is_virtual`, `is_const`, `is_noexcept`, `is_template` are inherited from the schema doc. `is_constexpr`, `is_override`, `is_final` are added by this contract; the rationale is below.

### Which symbols get a row

A row is emitted for **every C++ symbol** (`symbol.language = "cpp"`) regardless of whether any column is non-default. Rationale: a missing row is ambiguous (was the symbol skipped, or are all defaults true?). Always-present rows let queries join cleanly.

### Which symbols carry which columns

| Column | Applies to | Default for inapplicable symbols |
|---|---|---|
| `is_virtual` | methods, destructors | `false` |
| `is_const` | methods (const-qualified), constants (`const` storage on a variable/field) | `false` |
| `is_noexcept` | functions, methods, constructors, destructors | `false` |
| `is_template` | functions, methods, classes, structs, type aliases | `false` |
| `is_constexpr` | functions, methods, variables, fields | `false` |
| `is_override` | methods (only meaningful on methods) | `false` |
| `is_final` | classes, structs, methods | `false` |

For symbol kinds where a column does not apply (e.g. `is_override` on a class), the value is always the default. Querying `*cpp_attrs{symbol_id, is_override: true}` will never return a class.

### Why the three extra columns?

- **`is_constexpr`** — Constexpr functions are common in modern C++ (C++14 onward) and downstream queries care: compile-time-evaluable functions have very different security and complexity profiles. Cheap to detect (tree-sitter `storage_class_specifier`).
- **`is_override`** — The schema doc has `is_virtual`, but `is_virtual` is set on the *base* declaration. Overrides via the C++11 `override` contextual keyword are practically more useful for "find all overrides of X" queries. Cheap to detect (`virtual_specifier` child node).
- **`is_final`** — Symmetric with `is_override`. Sealed classes and methods influence virtual-dispatch reasoning. Cheap to detect (same `virtual_specifier` node, value `"final"`).

These three are added now rather than deferred because all three sit on the same AST nodes already being inspected for `is_virtual` / `is_const`.

---

## Extraction rules

For each column: which tree-sitter node or modifier produces a non-default value, and the default behavior when absent.

### `is_virtual`

- **AST source**: a `virtual_specifier` child of a `function_definition` / `field_declaration` / `declaration` with text `"virtual"`. In practice, the `virtual` keyword appears as a child token of the surrounding node before the return type.
- **Default**: `false`.
- **Pure-virtual** (`= 0`): `is_virtual = true`. (Abstractness is separately captured via `symbol.is_abstract = true`; the two columns agree by construction on pure-virtual methods.)
- **Implicitly virtual via override**: a method declared with `override` but no `virtual` keyword still overrides a virtual base method. We set `is_virtual = true` in this case, because semantically the method *is* virtual. (Detected by: presence of `virtual_specifier` with text `override`.) Override status is independently captured in `is_override`.
- **Edge case — out-of-line definition**: `int Foo::bar() { ... }` defined in a `.cpp` file does *not* repeat the `virtual` keyword. The definition is a separate `symbol` row from the in-class declaration. **Rule**: `is_virtual` is set from the *declaration* if visible in the same workspace; if only the definition is visible, `is_virtual = false`. This is a known limitation; a future "link definitions to declarations" pass can normalize.

### `is_const`

- **AST source — methods**: a `type_qualifier` child of a `function_declarator` with text `"const"` (i.e. `void foo() const;`). This means the method does not modify `*this`.
- **AST source — variables/fields**: a `type_qualifier` child of the type with text `"const"` (`const int x = 5;` or `static const Foo k;`).
- **Default**: `false`.
- **Pointers**: `const int*` (pointer to const) is *not* `is_const = true` on the variable; the pointee is const, not the pointer. `int* const` (const pointer) *is* `is_const = true`. Detection: the `const` qualifier must appear *after* the `*` in the declarator chain.
- **`constexpr` does not imply `is_const`**: they are independent. A `constexpr` function is not `const`; a `constexpr` variable is implicitly `const` *in C++ semantics*, but we still set `is_const` only when the keyword appears in source. Honesty over completeness.

### `is_noexcept`

- **AST source**: a `noexcept` keyword appearing as a child of the function signature, either bare (`void foo() noexcept;`) or with a constant-expression (`noexcept(true)`, `noexcept(noexcept(x))`).
- **Default**: `false`.
- **Bare `noexcept`**: `is_noexcept = true`.
- **`noexcept(true)`**: `is_noexcept = true`. We do not attempt to evaluate the expression; presence of `noexcept(...)` with any argument sets the bit. **`noexcept(false)` is the exception**: if the constant expression is the literal token `false`, `is_noexcept = false`. We pattern-match the literal — no expression evaluation.
- **Dynamic exception specifications** (`throw()`, deprecated since C++17): `is_noexcept = false`. We do not equate `throw()` with `noexcept` despite their similar semantics; the contract follows the syntactic keyword.

### `is_template`

- **AST source**: the symbol's defining node is wrapped in a `template_declaration` (the outer `template <...>` clause).
- **Default**: `false`.
- **Applies to**: function templates, class templates, struct templates, member templates, alias templates (`template <typename T> using Foo = ...;`).
- **Edge case — explicit specialization**: `template <> void foo<int>() { ... }` is *also* wrapped in `template_declaration` (empty parameter list). We set `is_template = true` for consistency. Queries that want unspecialized templates can additionally filter on template-parameter-list non-emptiness via a follow-up `template_params` relation (not in scope).
- **Edge case — variable templates** (C++14): `template <typename T> constexpr T pi = ...;` — `is_template = true` on the variable symbol.

### `is_constexpr`

- **AST source**: a `storage_class_specifier` child with text `"constexpr"`, or its presence as a token in the declaration's specifier list.
- **Default**: `false`.
- **`consteval` and `constinit`** (C++20): these are distinct from `constexpr` and do not set `is_constexpr`. They are not currently emitted (would need new columns).

### `is_override`

- **AST source**: a `virtual_specifier` child of the function declarator with text `"override"`.
- **Default**: `false`.
- **Edge case**: `override` is a contextual keyword; the C++ grammar treats it as a `virtual_specifier`. Detection: child node kind `virtual_specifier`, text comparison.
- **Both `override` and `final`**: both bits set independently. The combination `void foo() override final;` is valid C++.

### `is_final`

- **AST source — classes**: a `virtual_specifier` child of the `class_specifier` / `struct_specifier` (after the class name, before the base clause) with text `"final"`.
- **AST source — methods**: a `virtual_specifier` child of the function declarator with text `"final"`.
- **Default**: `false`.
- **Note**: `final` is contextual; tree-sitter still parses it as `virtual_specifier`.

---

## Worked examples

All examples are drawn from `../virgil-skills/benchmarks/cpp/data-processor/`.

### Example 1 — Virtual method with default attributes (Stage base class)

**Source:** `include/dataforge/core/stage.hpp:16-18`

```cpp
    virtual bool initialize(const std::map<std::string, std::string>& params);
    virtual int process(void* input, void* output);
    virtual void cleanup();
```

**`cpp_attrs` row for `Stage::initialize`** (`symbol_id = include/dataforge/core/stage.hpp|16|17|initialize|method`):

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `include/dataforge/core/stage.hpp\|16\|17\|initialize\|method` | true | false | false | false | false | false | false |

Detection: the `function_declarator` is preceded by a `virtual` keyword token in the surrounding `field_declaration` node. No other specifiers present.

Identical shape for `Stage::process` (L17) and `Stage::cleanup` (L18) — each row has only `is_virtual = true`.

### Example 2 — `is_const` on a const method

**Source:** `include/dataforge/core/pipeline.hpp:23-24`

```cpp
    std::string get_name() const;
    int stage_count() const;
```

**`cpp_attrs` rows:**

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `include/dataforge/core/pipeline.hpp\|23\|16\|get_name\|method` | false | true | false | false | false | false | false |
| `include/dataforge/core/pipeline.hpp\|24\|8\|stage_count\|method` | false | true | false | false | false | false | false |

Detection: the `function_declarator` has a trailing `type_qualifier` child whose text is `"const"`.

Note: the matching definitions in `src/core/pipeline.cpp:127-133` are *separate* symbol rows. By the "out-of-line definitions" rule, those rows would have `is_const = false` unless we cross-reference back to the declaration. The contract test asserts the declaration row; the definition row is a known limitation and tracked separately.

### Example 3 — `is_override` (non-obvious case via the `override` contextual keyword)

**Source:** `include/dataforge/io/csv_reader.hpp:14-19`

```cpp
    ~CsvReader() override;
    bool open() override;
    bool close() override;
    int read_batch(void* buffer, int max_records) override;
    bool has_more() const override;
```

**`cpp_attrs` row for `CsvReader::has_more`** (the non-obvious case: `const` *and* `override`):

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `include/dataforge/io/csv_reader.hpp\|19\|9\|has_more\|method` | true | true | false | false | false | true | false |

`is_virtual = true` here is the **non-obvious** part: the source does not contain the `virtual` keyword on this line. The method is virtual because (a) its base `Reader::has_more` is virtual, and (b) the `override` keyword forces virtual-ness. Per the extraction rule for `is_virtual`, the presence of an `override` `virtual_specifier` sets `is_virtual = true` even when the `virtual` keyword is absent in source.

The `CsvReader::~CsvReader() override` row (line 14) has `is_virtual = true, is_override = true, is_const = false, is_noexcept = false`. Same pattern: `override` implies `is_virtual`.

### Example 4 — `is_template` from a `template_declaration` wrapper

**Source:** synthesized minimal example (the data-processor benchmark contains no explicit `template` clauses; this example illustrates the contract for a construct the extractor must handle correctly when it appears elsewhere):

```cpp
// hypothetical_pool.hpp, line 1
template <typename T>
class Pool {
public:
    T* allocate();
    void release(T* item);
};
```

**`cpp_attrs` rows:**

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `hypothetical_pool.hpp\|2\|6\|Pool\|class` | false | false | false | true | false | false | false |
| `hypothetical_pool.hpp\|4\|7\|allocate\|method` | false | false | false | false | false | false | false |
| `hypothetical_pool.hpp\|5\|9\|release\|method` | false | false | false | false | false | false | false |

`is_template = true` on the *class* row because the `class_specifier` is the direct child of a `template_declaration`. The members `allocate` and `release` are *not* themselves templates — they are ordinary methods of a templated class. `is_template` is set only for the symbol that owns the `template <...>` clause.

(If the benchmark gains a `template <typename T>` construct, this example should be replaced with a real-source example.)

### Example 5 — `is_noexcept` and the `noexcept(false)` edge case

**Source:** synthesized (the benchmark does not use `noexcept`; this example illustrates the contract):

```cpp
// hypothetical_swap.hpp, line 3
void swap_fast(int& a, int& b) noexcept;
void swap_maybe(int& a, int& b) noexcept(true);
void swap_throws(int& a, int& b) noexcept(false);
```

**`cpp_attrs` rows:**

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `hypothetical_swap.hpp\|3\|5\|swap_fast\|function` | false | false | true | false | false | false | false |
| `hypothetical_swap.hpp\|4\|5\|swap_maybe\|function` | false | false | true | false | false | false | false |
| `hypothetical_swap.hpp\|5\|5\|swap_throws\|function` | false | false | false | false | false | false | false |

Detection: the `function_declarator` has a `noexcept` keyword child. For `swap_maybe`, the parenthesized argument's only token is `true`. For `swap_throws`, the only token is the literal `false`, which is the documented exception that forces `is_noexcept = false` without evaluating any expression.

(When the benchmark gains a `noexcept` declaration, this example should be replaced.)

### Example 6 — All-defaults row

**Source:** `include/dataforge/core/stage.hpp:11`

```cpp
class Stage {
```

**`cpp_attrs` row:**

| symbol_id | is_virtual | is_const | is_noexcept | is_template | is_constexpr | is_override | is_final |
|---|---|---|---|---|---|---|---|
| `include/dataforge/core/stage.hpp\|11\|6\|Stage\|class` | false | false | false | false | false | false | false |

A class with no `final`, no `template <>` wrapper, no other modifier. The row exists (every C++ symbol gets one) and every column is the default. This row is the contract that "missing-row vs. all-defaults" is unambiguous: it is always all-defaults.
