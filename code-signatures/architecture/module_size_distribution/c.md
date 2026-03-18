# Module Size Distribution -- C

## Overview
Module size distribution measures how symbol definitions are spread across source files in a C codebase. Balanced module sizes promote readability, reduce merge conflicts, and make it easier to reason about each compilation unit's responsibility. Extremely large or extremely small files both indicate structural problems worth investigating.

## Why It's an Architecture Concern
Oversized C files concentrate too many functions, structs, and macros into a single compilation unit, making the file difficult to navigate, slow to compile incrementally, and prone to merge conflicts when multiple developers work on it simultaneously. They often signal that responsibilities have not been properly decomposed into separate modules. Conversely, anemic modules that contain only a single symbol create unnecessary indirection -- a developer must open many files to follow even a simple code path, and the build system must manage many compilation units for little organizational benefit.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c, .h`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```c
// utils.c -- a dumping ground for unrelated utilities
#include "utils.h"

void parse_config(const char *path) { /* ... */ }
int validate_input(const char *input) { /* ... */ }
char *format_output(int code) { /* ... */ }
void log_message(const char *msg) { /* ... */ }
void send_packet(int fd, const void *data, size_t len) { /* ... */ }
int receive_packet(int fd, void *buf, size_t len) { /* ... */ }
// ... 25 more functions covering logging, networking, parsing, formatting
struct config_entry { /* ... */ };
struct log_context { /* ... */ };
typedef void (*callback_fn)(int);
#define MAX_RETRIES 5
#define BUFFER_SIZE 4096
```

### Good Code (Fix)
```c
// config.c -- focused on configuration parsing
#include "config.h"

struct config_entry {
    const char *key;
    const char *value;
};

void parse_config(const char *path) { /* ... */ }
int validate_config(const struct config_entry *entry) { /* ... */ }
```

```c
// network.c -- focused on network operations
#include "network.h"

void send_packet(int fd, const void *data, size_t len) { /* ... */ }
int receive_packet(int fd, void *buf, size_t len) { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `struct_specifier`, `union_specifier`, `enum_specifier`, `type_definition`, `preproc_function_def`, `preproc_def`, `declaration`
- **Detection approach**: Count all top-level symbol definitions (direct children of `translation_unit`). Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(translation_unit
  [
    (function_definition) @def
    (struct_specifier name: (_)) @def
    (union_specifier name: (_)) @def
    (enum_specifier name: (_)) @def
    (type_definition) @def
    (preproc_function_def) @def
    (preproc_def) @def
    (declaration) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```c
// utils.h -- massive public header
#ifndef UTILS_H
#define UTILS_H

void parse_config(const char *path);
int validate_input(const char *input);
char *format_output(int code);
void log_message(const char *msg);
void send_packet(int fd, const void *data, size_t len);
int receive_packet(int fd, void *buf, size_t len);
void init_cache(void);
void flush_cache(void);
int compress_data(const void *src, void *dst, size_t len);
// ... 15 more declarations spanning unrelated concerns
#endif
```

### Good Code (Fix)
```c
// config.h -- focused public interface
#ifndef CONFIG_H
#define CONFIG_H

void parse_config(const char *path);
int validate_config_entry(const char *key, const char *value);

#endif
```

```c
// network.h -- focused public interface
#ifndef NETWORK_H
#define NETWORK_H

void send_packet(int fd, const void *data, size_t len);
int receive_packet(int fd, void *buf, size_t len);

#endif
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `declaration`, `struct_specifier`, `type_definition`, `preproc_def`, `preproc_function_def`
- **Detection approach**: Count non-static top-level declarations and definitions. A symbol without the `static` storage class specifier is considered exported (external linkage). Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(translation_unit
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @name)) @def)

(translation_unit
  (declaration
    (storage_class_specifier) @storage) @decl)
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```c
// max_retries.c
#include "max_retries.h"

int get_max_retries(void) {
    return 5;
}
```

### Good Code (Fix)
```c
// config.c -- merge the trivial function into a related module
#include "config.h"

int get_max_retries(void) {
    return 5;
}

void parse_config(const char *path) { /* ... */ }
int validate_config_entry(const char *key, const char *value) { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `struct_specifier`, `union_specifier`, `enum_specifier`, `type_definition`, `preproc_function_def`, `preproc_def`, `declaration`
- **Detection approach**: Count top-level symbol definitions (direct children of `translation_unit`). Flag if count == 1, excluding files that are clearly entry points (contain `main`) or test files.
- **S-expression query sketch**:
```scheme
(translation_unit
  [
    (function_definition) @def
    (struct_specifier name: (_)) @def
    (union_specifier name: (_)) @def
    (enum_specifier name: (_)) @def
    (type_definition) @def
    (preproc_function_def) @def
    (preproc_def) @def
    (declaration) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
