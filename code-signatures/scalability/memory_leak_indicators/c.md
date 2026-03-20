# Memory Leak Indicators -- C

## Overview
Memory leaks in C are among the most critical scalability issues because there is no garbage collector. Every `malloc()`, `calloc()`, `fopen()`, `socket()`, or `strdup()` must have a corresponding cleanup call. Missing `free()`, `fclose()`, or `close()` causes permanent memory/resource leaks.

## Why It's a Scalability Concern
C programs (servers, daemons, embedded systems) often run for months or years. Each leaked allocation accumulates — a server leaking 1KB per request will consume 1GB after 1 million requests. Without cleanup, virtual memory grows until the OS kills the process or the system runs out of memory.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: stdlib, POSIX, libcurl, OpenSSL
- **Existing pipeline**: `memory_leaks.rs` in `src/audit/pipelines/c/` — extends with additional patterns

---

## Pattern 1: malloc()/calloc() Without free()

### Description
Allocating memory with `malloc()`, `calloc()`, or `realloc()` without a corresponding `free()` in the same function or a clear ownership transfer path.

### Bad Code (Anti-pattern)
```c
char *format_message(const char *prefix, const char *body) {
    char *result = malloc(strlen(prefix) + strlen(body) + 2);
    sprintf(result, "%s %s", prefix, body);
    return result;  // caller must free — but nothing enforces this
}

void process_messages(const char **messages, int count) {
    for (int i = 0; i < count; i++) {
        char *formatted = format_message("INFO", messages[i]);
        printf("%s\n", formatted);
        // forgot to free(formatted) — leaks on every iteration
    }
}
```

### Good Code (Fix)
```c
void process_messages(const char **messages, int count) {
    for (int i = 0; i < count; i++) {
        char *formatted = format_message("INFO", messages[i]);
        printf("%s\n", formatted);
        free(formatted);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `declaration`
- **Detection approach**: Find `call_expression` calling `malloc`, `calloc`, or `realloc` assigned to a local variable. Search the rest of the function for `free()` called with that variable. Flag if no `free()` or return of that variable exists. In loops, the variable from the previous iteration is overwritten, so the leak is per-iteration.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var_name)
    value: (call_expression
      function: (identifier) @func
      (#match? @func "^(malloc|calloc|realloc)$"))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `malloc_without_free`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: fopen() Without fclose()

### Description
Opening a file with `fopen()` without a corresponding `fclose()` on all code paths, leaking file descriptors.

### Bad Code (Anti-pattern)
```c
int count_lines(const char *filename) {
    FILE *fp = fopen(filename, "r");
    if (!fp) return -1;
    int count = 0;
    char buf[1024];
    while (fgets(buf, sizeof(buf), fp)) {
        count++;
    }
    return count;  // forgot fclose(fp)
}
```

### Good Code (Fix)
```c
int count_lines(const char *filename) {
    FILE *fp = fopen(filename, "r");
    if (!fp) return -1;
    int count = 0;
    char buf[1024];
    while (fgets(buf, sizeof(buf), fp)) {
        count++;
    }
    fclose(fp);
    return count;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `declaration`
- **Detection approach**: Find `call_expression` calling `fopen` assigned to a variable. Search the function for `fclose()` called with that variable. Flag if no `fclose()` exists.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var)
    value: (call_expression
      function: (identifier) @func
      (#eq? @func "fopen"))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `fopen_without_fclose`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: socket() Without close()

### Description
Creating a socket with `socket()` without a corresponding `close()` on all code paths, leaking file descriptors and network resources.

### Bad Code (Anti-pattern)
```c
int check_port(const char *host, int port) {
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    inet_pton(AF_INET, host, &addr.sin_addr);
    int result = connect(sock, (struct sockaddr *)&addr, sizeof(addr));
    return result == 0 ? 1 : 0;  // socket never closed
}
```

### Good Code (Fix)
```c
int check_port(const char *host, int port) {
    int sock = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr;
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    inet_pton(AF_INET, host, &addr.sin_addr);
    int result = connect(sock, (struct sockaddr *)&addr, sizeof(addr));
    close(sock);
    return result == 0 ? 1 : 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `declaration`
- **Detection approach**: Find `call_expression` calling `socket` assigned to a variable. Search the function for `close()` with that variable. Flag if no `close()` exists.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (identifier) @var
    value: (call_expression
      function: (identifier) @func
      (#eq? @func "socket"))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `socket_without_close`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: strdup()/asprintf() Without free()

### Description
Using `strdup()`, `strndup()`, or `asprintf()` which internally allocate memory, without freeing the result. These are easy to miss because the allocation is hidden inside the function call.

### Bad Code (Anti-pattern)
```c
void log_event(const char *event, const char *user) {
    char *msg;
    asprintf(&msg, "[%s] Event: %s by %s", timestamp(), event, user);
    write_log(msg);
    // msg never freed
}
```

### Good Code (Fix)
```c
void log_event(const char *event, const char *user) {
    char *msg;
    asprintf(&msg, "[%s] Event: %s by %s", timestamp(), event, user);
    write_log(msg);
    free(msg);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`
- **Detection approach**: Find `call_expression` calling `strdup`, `strndup` assigned to a variable, or `asprintf` with a pointer argument. Search the function for `free()` on that variable. Flag if no `free()` exists.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var)
    value: (call_expression
      function: (identifier) @func
      (#match? @func "^(strdup|strndup)$"))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `strdup_without_free`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: realloc in Loop Without Size Bound

### Description
Calling `realloc()` in a loop to grow a buffer without any maximum size check, causing unbounded memory allocation.

### Bad Code (Anti-pattern)
```c
char *read_all(int fd) {
    size_t capacity = 1024;
    size_t size = 0;
    char *buf = malloc(capacity);
    ssize_t n;
    while ((n = read(fd, buf + size, capacity - size)) > 0) {
        size += n;
        if (size == capacity) {
            capacity *= 2;
            buf = realloc(buf, capacity);  // unbounded growth
        }
    }
    return buf;
}
```

### Good Code (Fix)
```c
#define MAX_READ_SIZE (100 * 1024 * 1024)  // 100 MB limit

char *read_all(int fd) {
    size_t capacity = 1024;
    size_t size = 0;
    char *buf = malloc(capacity);
    ssize_t n;
    while ((n = read(fd, buf + size, capacity - size)) > 0) {
        size += n;
        if (size == capacity) {
            if (capacity * 2 > MAX_READ_SIZE) {
                free(buf);
                return NULL;
            }
            capacity *= 2;
            char *new_buf = realloc(buf, capacity);
            if (!new_buf) {
                free(buf);
                return NULL;
            }
            buf = new_buf;
        }
    }
    return buf;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `while_statement`, `for_statement`
- **Detection approach**: Find `call_expression` calling `realloc` inside a `while_statement` or `for_statement`. Check if there's a maximum size comparison or bounds check before the `realloc`. Flag if no size limit exists.
- **S-expression query sketch**:
```scheme
(while_statement
  body: (compound_statement
    (expression_statement
      (assignment_expression
        right: (call_expression
          function: (identifier) @func
          (#eq? @func "realloc"))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `realloc_unbounded`
- **Severity**: warning
- **Confidence**: medium
