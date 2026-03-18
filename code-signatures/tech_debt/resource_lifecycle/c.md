# Resource Lifecycle -- C

## Overview
Resources that are acquired but never properly released cause memory leaks, file descriptor exhaustion, and undefined behavior. In C, the most common manifestations are `malloc`/`calloc`/`realloc` allocations without corresponding `free` calls, and unchecked `malloc` return values that lead to null pointer dereferences.

## Why It's a Tech Debt Concern
C has no garbage collector, no RAII, and no automatic resource management. Every heap allocation must be explicitly freed by the programmer, and every allocation can fail by returning `NULL`. Memory leaks in long-running C programs (daemons, embedded systems, servers) cause steady memory growth until the process is killed by the OOM killer or the system becomes unresponsive. Unchecked `malloc` returns lead to null pointer dereferences that crash the program, or worse, cause silent data corruption when the null pointer is used in arithmetic or passed to other functions.

## Applicability
- **Relevance**: high (heap allocation is fundamental in C)
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: POSIX, any C library using dynamic allocation

---

## Pattern 1: malloc Without free -- Memory Leak

### Description
Allocating memory with `malloc`, `calloc`, or `realloc` and assigning the pointer to a local variable that goes out of scope without being freed. This includes early returns that bypass the `free` call, error paths that `goto` cleanup labels but miss some allocations, and functions that allocate memory and return it without documenting the caller's obligation to free.

### Bad Code (Anti-pattern)
```c
// Early return bypasses free
void process_data(const char *input) {
    char *buffer = malloc(1024);
    char *temp = malloc(512);

    if (parse(input, buffer) < 0) {
        free(buffer);
        return;  // temp is leaked
    }

    transform(buffer, temp);
    free(buffer);
    free(temp);
}

// Allocation in a loop without free
void process_files(const char **paths, int count) {
    for (int i = 0; i < count; i++) {
        char *content = malloc(MAX_FILE_SIZE);
        read_file(paths[i], content);
        analyze(content);
        // content never freed -- leaks on every iteration
    }
}

// Realloc leak -- original pointer lost on failure
void append_data(char **buf, size_t *size, const char *data, size_t len) {
    *buf = realloc(*buf, *size + len);
    // If realloc returns NULL, original *buf is leaked
    memcpy(*buf + *size, data, len);
    *size += len;
}
```

### Good Code (Fix)
```c
// All paths free all allocations
void process_data(const char *input) {
    char *buffer = malloc(1024);
    char *temp = malloc(512);

    if (!buffer || !temp) {
        free(buffer);
        free(temp);
        return;
    }

    if (parse(input, buffer) < 0) {
        free(buffer);
        free(temp);
        return;
    }

    transform(buffer, temp);
    free(buffer);
    free(temp);
}

// Free in every loop iteration
void process_files(const char **paths, int count) {
    for (int i = 0; i < count; i++) {
        char *content = malloc(MAX_FILE_SIZE);
        if (!content) continue;
        read_file(paths[i], content);
        analyze(content);
        free(content);
    }
}

// Safe realloc pattern
void append_data(char **buf, size_t *size, const char *data, size_t len) {
    char *new_buf = realloc(*buf, *size + len);
    if (!new_buf) {
        // Original *buf is still valid -- caller can free it
        return;
    }
    *buf = new_buf;
    memcpy(*buf + *size, data, len);
    *size += len;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `declaration`, `return_statement`
- **Detection approach**: Find `call_expression` nodes calling `malloc`, `calloc`, or `realloc` whose return value is assigned to a local pointer variable in a `declaration` or `assignment_expression`. Then scan the enclosing function body for `free()` calls with the same variable. Flag functions where a `return_statement` exists on a code path between the allocation and the `free` call without freeing the variable first. Also flag loop bodies containing `malloc` without a corresponding `free`.
- **S-expression query sketch**:
  ```scheme
  ;; malloc assigned to variable
  (declaration
    declarator: (init_declarator
      declarator: (pointer_declarator
        declarator: (identifier) @var_name)
      value: (call_expression
        function: (identifier) @alloc_fn)))

  ;; free call
  (call_expression
    function: (identifier) @free_fn
    arguments: (argument_list
      (identifier) @freed_var))
  ```

### Pipeline Mapping
- **Pipeline name**: `memory_leaks`
- **Pattern name**: `malloc_without_free`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Unchecked malloc Return Value

### Description
Calling `malloc`, `calloc`, or `realloc` without checking if the return value is `NULL`. When the system is out of memory, these functions return `NULL`, and dereferencing the result causes undefined behavior (typically a segmentation fault). In embedded systems and safety-critical code, this is especially dangerous.

### Bad Code (Anti-pattern)
```c
// Dereferencing potentially NULL pointer
void init_array(int **arr, int size) {
    *arr = malloc(size * sizeof(int));
    memset(*arr, 0, size * sizeof(int));  // UB if malloc returned NULL
}

// Struct allocation without check
struct Node *create_node(int value) {
    struct Node *node = malloc(sizeof(struct Node));
    node->value = value;  // Crash if NULL
    node->next = NULL;
    return node;
}

// String duplication without check
char *duplicate_string(const char *src) {
    size_t len = strlen(src) + 1;
    char *dst = malloc(len);
    strcpy(dst, src);  // UB if dst is NULL
    return dst;
}
```

### Good Code (Fix)
```c
// Check and handle NULL
int init_array(int **arr, int size) {
    *arr = malloc(size * sizeof(int));
    if (*arr == NULL) {
        return -1;  // Signal allocation failure to caller
    }
    memset(*arr, 0, size * sizeof(int));
    return 0;
}

// Check before use
struct Node *create_node(int value) {
    struct Node *node = malloc(sizeof(struct Node));
    if (node == NULL) {
        return NULL;
    }
    node->value = value;
    node->next = NULL;
    return node;
}

// Check and propagate failure
char *duplicate_string(const char *src) {
    size_t len = strlen(src) + 1;
    char *dst = malloc(len);
    if (dst == NULL) {
        return NULL;
    }
    memcpy(dst, src, len);
    return dst;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration`, `call_expression`, `identifier`, `if_statement`
- **Detection approach**: Find `call_expression` nodes calling `malloc`, `calloc`, or `realloc` whose return value is assigned to a variable. Check if the very next statement (or any statement before the variable is first dereferenced) is an `if_statement` checking the variable against `NULL`. Flag allocations where the variable is used (dereferenced via `->`, `*`, or passed to `memset`/`memcpy`) without a preceding null check.
- **S-expression query sketch**:
  ```scheme
  ;; Allocation assigned to variable
  (declaration
    declarator: (init_declarator
      declarator: (pointer_declarator
        declarator: (identifier) @var_name)
      value: (call_expression
        function: (identifier) @alloc_fn)))

  ;; Null check (safe pattern)
  (if_statement
    condition: (parenthesized_expression
      (binary_expression
        left: (identifier) @checked_var
        right: (null))))
  ```

### Pipeline Mapping
- **Pipeline name**: `unchecked_malloc`
- **Pattern name**: `unchecked_alloc_return`
- **Severity**: warning
- **Confidence**: high
