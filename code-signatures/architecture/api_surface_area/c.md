# API Surface Area -- C

## Overview
API surface area in C is determined by what symbols are visible across translation units. Since C lacks access modifiers on struct fields, the primary mechanism is the `static` keyword, which restricts linkage to the current file. Header files that declare many non-static functions create a wide public API that any consumer can depend on, making changes costly and error-prone.

## Why It's an Architecture Concern
In C, every non-static function declared in a header becomes part of the module's public contract. A large public API creates tight coupling between translation units: callers depend on function signatures, struct layouts, and constant definitions that become difficult to change without cascading updates. Excessive exposure also increases the risk of name collisions in the global namespace and makes it harder to reason about which functions are safe to refactor. Keeping the public surface minimal by marking internal helpers as `static` and using opaque pointers reduces coupling and simplifies maintenance.

## Applicability
- **Relevance**: low
- **Languages covered**: `.c, .h`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```c
/* utils.h */
#ifndef UTILS_H
#define UTILS_H

int parse_header(const char *buf);
int parse_body(const char *buf);
int parse_footer(const char *buf);
int validate_checksum(const char *buf);
int compute_offset(int base, int delta);
int encode_payload(char *out, const char *in);
int decode_payload(char *out, const char *in);
int compress_block(char *out, const char *in);
int decompress_block(char *out, const char *in);
int format_output(char *out, int len);
int flush_buffer(char *buf, int len);

#endif
```

### Good Code (Fix)
```c
/* utils.h */
#ifndef UTILS_H
#define UTILS_H

/* Public API — only what consumers actually need */
int parse_message(const char *buf, char *out);
int encode_payload(char *out, const char *in);
int decode_payload(char *out, const char *in);

#endif

/* utils.c */
#include "utils.h"

static int parse_header(const char *buf)  { /* ... */ }
static int parse_body(const char *buf)    { /* ... */ }
static int parse_footer(const char *buf)  { /* ... */ }
static int validate_checksum(const char *buf) { /* ... */ }
static int compute_offset(int base, int delta) { /* ... */ }
static int compress_block(char *out, const char *in) { /* ... */ }
static int decompress_block(char *out, const char *in) { /* ... */ }
static int format_output(char *out, int len) { /* ... */ }
static int flush_buffer(char *buf, int len) { /* ... */ }

int parse_message(const char *buf, char *out) {
    /* orchestrates static helpers */
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `declaration` (in headers)
- **Detection approach**: Count total top-level function declarations and definitions. A symbol is exported if it lacks the `static` storage class specifier. Flag if total >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match non-static function declarations (exported)
(declaration
  declarator: (function_declarator
    declarator: (identifier) @func.name)) @exported

;; Match static function definitions (not exported)
(function_definition
  (storage_class_specifier) @storage
  declarator: (function_declarator
    declarator: (identifier) @static.func.name)
  (#eq? @storage "static"))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```c
/* connection.h */
#ifndef CONNECTION_H
#define CONNECTION_H

#include <netinet/in.h>

typedef struct {
    int socket_fd;
    struct sockaddr_in addr;
    char recv_buffer[4096];
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
} Connection;

Connection *connection_new(const char *host, int port);
int connection_send(Connection *conn, const char *data);

#endif
```

### Good Code (Fix)
```c
/* connection.h — opaque pointer hides internals */
#ifndef CONNECTION_H
#define CONNECTION_H

typedef struct Connection Connection;

Connection *connection_new(const char *host, int port);
void        connection_free(Connection *conn);
int         connection_send(Connection *conn, const char *data);
int         connection_is_connected(const Connection *conn);

#endif

/* connection.c — struct definition is private */
#include "connection.h"
#include <netinet/in.h>

struct Connection {
    int socket_fd;
    struct sockaddr_in addr;
    char recv_buffer[4096];
    int buffer_pos;
    int retry_count;
    int max_retries;
    int is_connected;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_specifier`, `type_definition` with `struct_specifier`
- **Detection approach**: Find struct definitions in header files that are fully defined (not opaque forward declarations). A struct with many fields declared in a header leaks its layout to all consumers. Flag structs with 4+ fields defined in `.h` files.
- **S-expression query sketch**:
```scheme
;; Match struct definitions with visible field lists in headers
(type_definition
  type: (struct_specifier
    body: (field_declaration_list
      (field_declaration
        declarator: (field_identifier) @field.name)))
  declarator: (type_identifier) @struct.name)
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
