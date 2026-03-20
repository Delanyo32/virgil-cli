# Legacy API Usage -- C

## Overview
Legacy API usage in C refers to relying on older preprocessor-heavy patterns and type-obscuring conventions when safer, more debuggable, and more maintainable alternatives exist. Common examples include using `#define` macros for constants and simple functions instead of `static inline` functions, and using `typedef` to hide pointer types behind opaque names.

## Why It's a Tech Debt Concern
`#define` macros bypass the type system entirely -- they perform textual substitution with no type checking, no scope boundaries, and no debugger visibility. Bugs in macro arguments (double evaluation, missing parentheses) are notoriously difficult to diagnose. `typedef`-hidden pointers obscure ownership semantics and `const` correctness -- a developer reading `MyHandle h` has no idea whether `h` is a value, a pointer, or a double pointer, leading to incorrect memory management and dangling pointer bugs. Both patterns accumulate because they are "traditional C" and rarely challenged during review.

## Applicability
- **Relevance**: high (both patterns are pervasive in C codebases of all ages)
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: #define Macros Instead of Inline Functions

### Description
Using `#define` preprocessor macros to define constant expressions or small utility functions instead of `static inline` functions or `enum`/`const` values. Macros perform textual substitution without type checking, can evaluate arguments multiple times (causing side effects), and are invisible to debuggers.

### Bad Code (Anti-pattern)
```c
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define CLAMP(x, lo, hi) (MAX(lo, MIN(hi, x)))
#define SQUARE(x) ((x) * (x))
#define IS_POWER_OF_TWO(x) (((x) != 0) && (((x) & ((x) - 1)) == 0))
#define ARRAY_SIZE(arr) (sizeof(arr) / sizeof((arr)[0]))
#define SWAP(a, b) do { typeof(a) _tmp = (a); (a) = (b); (b) = _tmp; } while(0)

#define MAX_BUFFER_SIZE 4096
#define DEFAULT_TIMEOUT 30
#define ERROR_INVALID_INPUT -1
#define ERROR_OUT_OF_MEMORY -2

void process(int *data, int n) {
    for (int i = 0; i < n; i++) {
        // Bug: if data[i]++ is passed, MAX evaluates it twice
        data[i] = MAX(data[i], 0);
        data[i] = CLAMP(data[i], 0, MAX_BUFFER_SIZE);
    }
}
```

### Good Code (Fix)
```c
#include <stddef.h>

enum { MAX_BUFFER_SIZE = 4096 };
enum { DEFAULT_TIMEOUT = 30 };

enum error_code {
    ERROR_INVALID_INPUT = -1,
    ERROR_OUT_OF_MEMORY = -2,
};

static inline int max_int(int a, int b) {
    return a > b ? a : b;
}

static inline int min_int(int a, int b) {
    return a < b ? a : b;
}

static inline int clamp_int(int x, int lo, int hi) {
    return max_int(lo, min_int(hi, x));
}

static inline int square_int(int x) {
    return x * x;
}

static inline int is_power_of_two(unsigned int x) {
    return x != 0 && (x & (x - 1)) == 0;
}

// ARRAY_SIZE is acceptable as a macro (sizeof is compile-time, no side effects)
#define ARRAY_SIZE(arr) (sizeof(arr) / sizeof((arr)[0]))

void process(int *data, int n) {
    for (int i = 0; i < n; i++) {
        // Safe: arguments evaluated exactly once
        data[i] = max_int(data[i], 0);
        data[i] = clamp_int(data[i], 0, MAX_BUFFER_SIZE);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_function_def` (function-like macros), `preproc_def` (object-like macros with numeric values)
- **Detection approach**: Find `preproc_function_def` nodes -- these are function-like macros that could be `static inline` functions. Also find `preproc_def` nodes whose value is a numeric literal or simple expression (candidates for `enum` or `const`). Exclude include guards (`#define HEADER_H`), feature flags, and platform detection macros. Flag function-like macros that reference their parameters more than once (double evaluation risk).
- **S-expression query sketch**:
```scheme
(preproc_function_def
  name: (identifier) @macro_name
  parameters: (preproc_params) @params
  value: (preproc_arg) @body)

(preproc_def
  name: (identifier) @const_name
  value: (preproc_arg) @const_value)
```

### Pipeline Mapping
- **Pipeline name**: `define_instead_of_inline`
- **Pattern name**: `function_like_macro`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: typedef Hiding Pointer Types

### Description
Using `typedef` to create aliases that hide the fact that a type is a pointer. This obscures ownership semantics, prevents correct use of `const`, and makes it impossible to tell from a variable declaration whether the variable is a value or a heap-allocated pointer that needs to be freed.

### Bad Code (Anti-pattern)
```c
typedef struct node *Node;
typedef struct connection *Connection;
typedef struct buffer *Buffer;
typedef char *String;
typedef void *Handle;

// Reader has no idea these are pointers
Node create_node(int value) {
    Node n = malloc(sizeof(struct node));
    n->value = value;
    n->next = NULL;
    return n;
}

void process(Connection conn, Buffer buf) {
    // Is conn a value or a pointer? Does buf need to be freed?
    // The typedef hides this critical information
    String name = get_name(conn);
    // Is name heap-allocated? Should the caller free it?
    printf("Name: %s\n", name);
}

// const doesn't do what you'd expect
void read_only(const Node n) {
    // This makes the POINTER const, not the data
    // n->value = 42;  // This compiles! The data is still mutable
    // n = other_node;  // Only this is prevented
}
```

### Good Code (Fix)
```c
struct node {
    int value;
    struct node *next;
};

struct connection {
    int fd;
    char *host;
};

struct buffer {
    char *data;
    size_t len;
    size_t cap;
};

// Pointer nature is explicit -- ownership is clear
struct node *create_node(int value) {
    struct node *n = malloc(sizeof(struct node));
    if (!n) return NULL;
    n->value = value;
    n->next = NULL;
    return n;
}

void process(struct connection *conn, struct buffer *buf) {
    // Clearly pointers -- caller knows they manage memory
    const char *name = get_name(conn);
    printf("Name: %s\n", name);
}

// const correctness works as expected
void read_only(const struct node *n) {
    // n->value = 42;  // Compiler error: data is const
    // n = other_node;  // Allowed: pointer itself is not const
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_definition` where the declarator is a `pointer_declarator`
- **Detection approach**: Find `type_definition` nodes whose declarator contains a `pointer_declarator` -- this means the typedef is hiding a pointer behind a plain name. Flag all occurrences. Exclude opaque handle patterns where the struct definition is deliberately hidden (common in public API headers), though these should still be reviewed.
- **S-expression query sketch**:
```scheme
(type_definition
  type: (_) @base_type
  declarator: (pointer_declarator
    declarator: (type_identifier) @alias_name))
```

### Pipeline Mapping
- **Pipeline name**: `typedef_pointer_hiding`
- **Pattern name**: `pointer_typedef`
- **Severity**: warning
- **Confidence**: medium
