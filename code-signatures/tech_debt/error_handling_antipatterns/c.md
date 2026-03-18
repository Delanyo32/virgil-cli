# Error Handling Anti-patterns -- C

## Overview
Errors that are silently ignored or left unchecked make debugging impossible and hide real failures. In C, where there are no exceptions, error handling relies on return values and `errno` -- both of which are routinely ignored, leading to silent failures and undefined behavior.

## Why It's a Tech Debt Concern
Ignoring return values from functions like `fopen`, `malloc`, `write`, and `close` means the program continues operating on invalid handles, null pointers, or partial I/O without any indication of failure. Unchecked `errno` after system calls means file permission errors, disk-full conditions, and network failures pass silently. In production, these ignored errors compound into data corruption, security vulnerabilities (null pointer dereferences, use-after-free), and crashes that are nearly impossible to reproduce.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c`, `.h`

---

## Pattern 1: Ignored Return Value

### Description
Calling a function that returns an error code or error-indicating value (NULL, -1, EOF) without checking the return value. Common with `malloc`, `fopen`, `fclose`, `write`, `read`, `snprintf`, and POSIX functions. The program proceeds with invalid state.

### Bad Code (Anti-pattern)
```c
void process_file(const char *path) {
    FILE *fp = fopen(path, "r");
    // fp could be NULL, but we use it directly
    char buffer[1024];
    fread(buffer, 1, sizeof(buffer), fp);
    fclose(fp);
}

void allocate_buffer(size_t size) {
    char *buf = malloc(size);
    // buf could be NULL
    memset(buf, 0, size);
    strcpy(buf, "initial data");
}

void write_data(int fd, const char *data, size_t len) {
    write(fd, data, len);
    // Partial write or failure completely ignored
    close(fd);
    // close() failure ignored
}
```

### Good Code (Fix)
```c
int process_file(const char *path) {
    FILE *fp = fopen(path, "r");
    if (fp == NULL) {
        fprintf(stderr, "Failed to open %s: %s\n", path, strerror(errno));
        return -1;
    }
    char buffer[1024];
    size_t n = fread(buffer, 1, sizeof(buffer), fp);
    if (n == 0 && ferror(fp)) {
        fprintf(stderr, "Failed to read %s: %s\n", path, strerror(errno));
        fclose(fp);
        return -1;
    }
    if (fclose(fp) != 0) {
        fprintf(stderr, "Failed to close %s: %s\n", path, strerror(errno));
        return -1;
    }
    return 0;
}

int allocate_buffer(size_t size, char **out) {
    char *buf = malloc(size);
    if (buf == NULL) {
        return -1;
    }
    memset(buf, 0, size);
    strncpy(buf, "initial data", size - 1);
    *out = buf;
    return 0;
}

int write_data(int fd, const char *data, size_t len) {
    ssize_t written = write(fd, data, len);
    if (written < 0) {
        perror("write failed");
        return -1;
    }
    if ((size_t)written < len) {
        fprintf(stderr, "Partial write: %zd of %zu bytes\n", written, len);
        return -1;
    }
    if (close(fd) < 0) {
        perror("close failed");
        return -1;
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `expression_statement`, `call_expression`, `identifier`
- **Detection approach**: Find `expression_statement` nodes containing a `call_expression` where the function name is a known error-returning function (`fopen`, `malloc`, `fclose`, `write`, `read`, `close`, `fwrite`, `fread`, `send`, `recv`, `pthread_create`, etc.). These are calls whose return value is not assigned to any variable. Also flag `declaration` nodes where the initializer is a `call_expression` to `malloc`/`fopen` but no subsequent `if` statement checks the declared variable against `NULL`.
- **S-expression query sketch**:
```scheme
;; Bare function call as a statement (return value discarded)
(expression_statement
  (call_expression
    function: (identifier) @func_name
    arguments: (argument_list)))

;; Assignment without subsequent NULL check
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var_name)
    value: (call_expression
      function: (identifier) @func_name)))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `ignored_return_value`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Unchecked errno

### Description
Performing operations that set `errno` on failure (system calls, math functions, `strtol`, etc.) without checking `errno` afterwards. The error condition passes silently and the program continues with potentially invalid results.

### Bad Code (Anti-pattern)
```c
long parse_number(const char *str) {
    long val = strtol(str, NULL, 10);
    // errno could be ERANGE for overflow, val could be 0 for invalid input
    return val;
}

void create_directory(const char *path) {
    mkdir(path, 0755);
    // EEXIST, EACCES, ENOENT -- all silently ignored
    chown(path, uid, gid);
    // Permission error silently ignored
}

void set_socket_option(int sockfd) {
    int optval = 1;
    setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &optval, sizeof(optval));
    // Failure to set socket option silently ignored
}
```

### Good Code (Fix)
```c
int parse_number(const char *str, long *out) {
    char *endptr;
    errno = 0;
    long val = strtol(str, &endptr, 10);
    if (errno == ERANGE) {
        fprintf(stderr, "Number out of range: %s\n", str);
        return -1;
    }
    if (endptr == str || *endptr != '\0') {
        fprintf(stderr, "Invalid number: %s\n", str);
        return -1;
    }
    *out = val;
    return 0;
}

int create_directory(const char *path, uid_t uid, gid_t gid) {
    if (mkdir(path, 0755) < 0) {
        if (errno != EEXIST) {
            fprintf(stderr, "mkdir %s failed: %s\n", path, strerror(errno));
            return -1;
        }
    }
    if (chown(path, uid, gid) < 0) {
        fprintf(stderr, "chown %s failed: %s\n", path, strerror(errno));
        return -1;
    }
    return 0;
}

int set_socket_option(int sockfd) {
    int optval = 1;
    if (setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &optval, sizeof(optval)) < 0) {
        perror("setsockopt SO_REUSEADDR failed");
        return -1;
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `expression_statement`, `call_expression`, `identifier`, `declaration`
- **Detection approach**: Find `call_expression` nodes for errno-setting functions (`strtol`, `strtod`, `strtoul`, `mkdir`, `chown`, `setsockopt`, `bind`, `listen`, etc.) where neither the immediately following statement nor the surrounding block contains a reference to `errno` or a comparison of the return value against an error sentinel (`-1`, `NULL`, `0`). Requires checking the next sibling statement in the parent block.
- **S-expression query sketch**:
```scheme
;; strtol call without errno check
(declaration
  declarator: (init_declarator
    declarator: (identifier) @var_name
    value: (call_expression
      function: (identifier) @func_name
      arguments: (argument_list))))

;; Bare call to errno-setting function
(expression_statement
  (call_expression
    function: (identifier) @func_name
    arguments: (argument_list)))
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `unchecked_errno`
- **Severity**: warning
- **Confidence**: medium
