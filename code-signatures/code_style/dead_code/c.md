# Dead Code -- C

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. It also increases compilation time and complicates refactoring. In C, dead code can also lurk inside `#ifdef` blocks that are never compiled for the target platform.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Static Function

### Description
A `static` function defined but never called from anywhere in the translation unit. Since `static` restricts linkage to the file, the compiler can determine it is unused — GCC/Clang warn with `-Wunused-function`, but warnings are often suppressed or ignored.

### Bad Code (Anti-pattern)
```c
static int clamp_legacy(int value, int min, int max)
{
    if (value < min) return min;
    if (value > max) return max;
    return value;
}

static int clamp(int value, int min, int max)
{
    return value < min ? min : (value > max ? max : value);
}

int normalize(int value)
{
    return clamp(value, 0, 255);
}
```

### Good Code (Fix)
```c
static int clamp(int value, int min, int max)
{
    return value < min ? min : (value > max ? max : value);
}

int normalize(int value)
{
    return clamp(value, 0, 255);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` with `storage_class_specifier` child of `static`
- **Detection approach**: Collect all `static` function definitions and their names. Cross-reference with all `call_expression` nodes in the same file (since `static` functions have file scope). Static functions with zero call sites are candidates. Exclude functions used as function pointers (address-of via `&function_name` or passed as arguments), functions in header files intended as `static inline`, and functions guarded by `#ifdef` that may be compiled conditionally.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @fn_name)
    (storage_class_specifier) @storage
    (#eq? @storage "static"))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Exit/Abort

### Description
Code statements that appear after an unconditional return, `exit()`, `abort()`, or `_Exit()` — they can never execute.

### Bad Code (Anti-pattern)
```c
int parse_port(const char *str)
{
    long val = strtol(str, NULL, 10);
    if (val <= 0 || val > 65535) {
        fprintf(stderr, "Invalid port: %s\n", str);
        exit(EXIT_FAILURE);
        return -1; /* unreachable — exit terminates the process */
    }
    return (int)val;
}

void fatal(const char *msg)
{
    fprintf(stderr, "FATAL: %s\n", msg);
    abort();
    fprintf(stderr, "This message never prints\n"); /* unreachable */
}
```

### Good Code (Fix)
```c
int parse_port(const char *str)
{
    long val = strtol(str, NULL, 10);
    if (val <= 0 || val > 65535) {
        fprintf(stderr, "Invalid port: %s\n", str);
        exit(EXIT_FAILURE);
    }
    return (int)val;
}

void fatal(const char *msg)
{
    fprintf(stderr, "FATAL: %s\n", msg);
    abort();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `call_expression` (for `exit`, `abort`, `_Exit`, `longjmp`)
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `compound_statement`. In C, `exit()`, `_Exit()`, `abort()`, and `longjmp()` are noreturn functions. Also check for statements after unconditional `return`. Exclude statements after `goto` labels (they may be jump targets) and statements inside `do { } while(0)` macros.
- **S-expression query sketch**:
  ```scheme
  (compound_statement
    (return_statement) @exit
    .
    (_) @unreachable)
  (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @fn_name
        (#any-of? @fn_name "exit" "abort" "_Exit"))) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Dead #ifdef Blocks

### Description
Preprocessor-guarded blocks (`#ifdef`, `#if 0`) that are never compiled because the macro is never defined or the condition is always false. This is a C-specific form of dead code that tree-sitter can partially detect.

### Bad Code (Anti-pattern)
```c
#if 0
/* Old implementation before we switched to OpenSSL */
static int compute_hmac(const unsigned char *key, size_t key_len,
                        const unsigned char *data, size_t data_len,
                        unsigned char *out)
{
    /* custom HMAC implementation ... 40 lines ... */
    return 0;
}
#endif

int compute_hmac(const unsigned char *key, size_t key_len,
                 const unsigned char *data, size_t data_len,
                 unsigned char *out)
{
    return HMAC(EVP_sha256(), key, key_len, data, data_len, out, NULL) != NULL;
}
```

### Good Code (Fix)
```c
int compute_hmac(const unsigned char *key, size_t key_len,
                 const unsigned char *data, size_t data_len,
                 unsigned char *out)
{
    return HMAC(EVP_sha256(), key, key_len, data, data_len, out, NULL) != NULL;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_if`, `preproc_ifdef`
- **Detection approach**: Find `preproc_if` nodes with condition `0` (literal false), which are definitively dead. For `preproc_ifdef`, check if the macro name is never `#define`d in the project. Also detect `#if 0` ... `#endif` patterns that wrap large code blocks. Exclude feature-toggle macros (e.g., `DEBUG`, `NDEBUG`, platform macros like `_WIN32`) that are defined by the build system.
- **S-expression query sketch**:
  ```scheme
  (preproc_if
    condition: (number_literal) @cond
    (#eq? @cond "0"))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `dead_ifdef_block`
- **Severity**: info
- **Confidence**: medium
