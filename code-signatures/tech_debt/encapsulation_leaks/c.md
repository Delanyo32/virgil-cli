# Encapsulation Leaks -- C

## Overview
Encapsulation leaks in C occur when global mutable variables declared with `extern` linkage are modified across multiple translation units, creating hidden coupling between source files, or when functions and local pointers omit `const` qualifiers, failing to communicate and enforce immutability contracts. Both patterns make C code harder to reason about, optimize, and safely modify.

## Why It's a Tech Debt Concern
Global mutable state shared via `extern` creates invisible dependencies between translation units — any `.c` file that includes the header can silently modify the variable, making it impossible to determine which code is responsible for a state change without reading the entire codebase. This defeats static analysis and makes concurrent execution hazardous. Missing `const` qualifiers strip away the compiler's ability to catch accidental mutations, prevent optimization opportunities, and force readers to manually trace whether a pointer target is modified.

## Applicability
- **Relevance**: high (extern globals and missing const are extremely common in C codebases)
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: Linux kernel (global subsystem state), embedded systems (hardware register globals), POSIX applications (global configuration)

---

## Pattern 1: Global Mutable State

### Description
A variable is declared with `extern` linkage in a header and defined in one translation unit, then read and modified by functions in multiple other translation units. The variable's value at any point depends on the order of function calls across the entire program, making behavior non-deterministic in concurrent contexts.

### Bad Code (Anti-pattern)
```c
/* app_state.h */
extern int g_log_level;
extern char g_config_path[PATH_MAX];
extern int g_request_count;
extern int g_error_count;
extern char *g_last_error;
extern int g_max_connections;

/* app_state.c */
int g_log_level = LOG_INFO;
char g_config_path[PATH_MAX] = "/etc/app/config.ini";
int g_request_count = 0;
int g_error_count = 0;
char *g_last_error = NULL;
int g_max_connections = 100;

/* server.c */
#include "app_state.h"

int handle_request(int client_fd) {
    g_request_count++;  /* race condition */
    if (g_log_level >= LOG_DEBUG)
        log_debug("Request #%d", g_request_count);
    /* ... */
    if (error) {
        g_error_count++;
        g_last_error = "connection failed";  /* dangling pointer risk */
    }
    return 0;
}

/* config.c */
#include "app_state.h"

int reload_config(void) {
    FILE *f = fopen(g_config_path, "r");
    /* ... */
    g_log_level = new_level;         /* changes server behavior */
    g_max_connections = new_max;     /* no synchronization */
    return 0;
}

/* monitor.c */
#include "app_state.h"

void print_stats(void) {
    printf("Requests: %d, Errors: %d\n", g_request_count, g_error_count);
    if (g_last_error)
        printf("Last error: %s\n", g_last_error);  /* might be stale */
    g_request_count = 0;  /* reset from another TU */
    g_error_count = 0;
}
```

### Good Code (Fix)
```c
/* app_state.h */
typedef struct {
    int log_level;
    char config_path[PATH_MAX];
    int request_count;
    int error_count;
    char last_error[256];
    int max_connections;
    pthread_mutex_t lock;
} AppState;

int app_state_init(AppState *state, const char *config_path);
void app_state_destroy(AppState *state);

int app_state_get_log_level(const AppState *state);
void app_state_set_log_level(AppState *state, int level);
void app_state_increment_requests(AppState *state);
void app_state_record_error(AppState *state, const char *msg);
void app_state_get_stats(const AppState *state, int *requests, int *errors);
void app_state_reset_stats(AppState *state);

/* app_state.c */
int app_state_init(AppState *state, const char *config_path) {
    memset(state, 0, sizeof(*state));
    state->log_level = LOG_INFO;
    state->max_connections = 100;
    strncpy(state->config_path, config_path, PATH_MAX - 1);
    pthread_mutex_init(&state->lock, NULL);
    return 0;
}

void app_state_increment_requests(AppState *state) {
    pthread_mutex_lock(&state->lock);
    state->request_count++;
    pthread_mutex_unlock(&state->lock);
}

void app_state_record_error(AppState *state, const char *msg) {
    pthread_mutex_lock(&state->lock);
    state->error_count++;
    strncpy(state->last_error, msg, sizeof(state->last_error) - 1);
    pthread_mutex_unlock(&state->lock);
}

/* server.c */
int handle_request(AppState *state, int client_fd) {
    app_state_increment_requests(state);
    /* ... */
    if (error)
        app_state_record_error(state, "connection failed");
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration` at `translation_unit` level with `extern` storage class specifier, or non-`static` file-scope variable definitions
- **Detection approach**: Find top-level `declaration` nodes with `storage_class_specifier` "extern" — these are global variable declarations visible across translation units. Also find top-level variable definitions (non-`static`, non-`const`) that represent mutable global state. Flag variables that appear in multiple translation units or header files with `extern` linkage.
- **S-expression query sketch**:
  ```scheme
  (translation_unit
    (declaration
      (storage_class_specifier) @storage
      declarator: (identifier) @global_name))

  (translation_unit
    (declaration
      type: (_) @type
      declarator: (init_declarator
        declarator: (identifier) @global_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `global_mutable_state`
- **Pattern name**: `extern_mutable_global`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Missing Const Qualifiers

### Description
Function parameters that are pointers to data the function does not modify lack `const` qualification, and local pointer variables that only read through the pointer are declared without `const`. This hides the function's actual contract from both the compiler and the reader, and prevents the compiler from catching accidental modifications.

### Bad Code (Anti-pattern)
```c
/* Pointer parameters not marked const even though data is only read */
int calculate_checksum(char *data, size_t len) {
    int checksum = 0;
    for (size_t i = 0; i < len; i++) {
        checksum ^= data[i];  /* only reads data */
    }
    return checksum;
}

int find_in_array(int *arr, size_t len, int target) {
    for (size_t i = 0; i < len; i++) {
        if (arr[i] == target)  /* only reads arr */
            return (int)i;
    }
    return -1;
}

void print_user(User *user) {
    printf("Name: %s, Email: %s\n", user->name, user->email);  /* only reads user */
}

int compare_buffers(char *buf1, char *buf2, size_t len) {
    return memcmp(buf1, buf2, len);  /* only reads both buffers */
}

void process_config(Config *config) {
    /* Local pointer that only reads */
    char *name = config->app_name;
    int port = config->port;
    printf("Starting %s on port %d\n", name, port);
    /* config is never modified */
}

/* Caller cannot pass const data */
const char *message = "hello";
// calculate_checksum(message, 5);  /* compiler warning/error */
```

### Good Code (Fix)
```c
int calculate_checksum(const char *data, size_t len) {
    int checksum = 0;
    for (size_t i = 0; i < len; i++) {
        checksum ^= data[i];
    }
    return checksum;
}

int find_in_array(const int *arr, size_t len, int target) {
    for (size_t i = 0; i < len; i++) {
        if (arr[i] == target)
            return (int)i;
    }
    return -1;
}

void print_user(const User *user) {
    printf("Name: %s, Email: %s\n", user->name, user->email);
}

int compare_buffers(const char *buf1, const char *buf2, size_t len) {
    return memcmp(buf1, buf2, len);
}

void process_config(const Config *config) {
    const char *name = config->app_name;
    int port = config->port;
    printf("Starting %s on port %d\n", name, port);
}

/* Caller can now pass const data */
const char *message = "hello";
calculate_checksum(message, 5);  /* works correctly */
```

### Tree-sitter Detection Strategy
- **Target node types**: `parameter_declaration` with `pointer_declarator` inside `function_definition`, checking for absence of `const` `type_qualifier`
- **Detection approach**: Find `parameter_declaration` nodes in `function_definition` parameter lists where the declarator is a `pointer_declarator` but the type specifier does not include a `const` `type_qualifier`. Then analyze the function body to determine if the pointer is only read (no assignment through the pointer, no passed to non-const parameter). Flag parameters where the pointer is never written through. Simpler heuristic: flag all non-const pointer parameters in functions under a configurable line count.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @func_name
      parameters: (parameter_list
        (parameter_declaration
          type: (type_identifier) @param_type
          declarator: (pointer_declarator
            declarator: (identifier) @param_name)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `missing_const`
- **Pattern name**: `non_const_read_only_pointer`
- **Severity**: info
- **Confidence**: medium
