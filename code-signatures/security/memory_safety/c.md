# Memory Safety -- C

## Overview
C provides no memory safety guarantees -- the programmer is fully responsible for bounds checking, memory allocation sizing, and initialization. Buffer overflows, integer overflows leading to memory corruption, and use of uninitialized memory are among the most exploited vulnerability classes in C codebases, consistently appearing in CVE databases year after year.

## Why It's a Security Concern
C is used in operating system kernels, embedded systems, network daemons, and security-critical libraries. Memory safety vulnerabilities in C code lead to arbitrary code execution, privilege escalation, and information disclosure. Buffer overflows via `strcpy`/`sprintf`/`gets` are the classic exploitation vector. Integer overflows in `malloc` size calculations create heap overflows. Uninitialized memory reads leak sensitive data from the stack or heap.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libc (string.h, stdlib.h, stdio.h), POSIX, OpenSSL, zlib

---

## Pattern 1: Buffer Overflow via Unsafe String Functions

### Description
Using `strcpy()`, `strcat()`, `sprintf()`, `gets()`, or `scanf("%s")` which perform no bounds checking. These functions write until they encounter a null terminator or format completion, regardless of the destination buffer size. If the source data is longer than the destination buffer, adjacent memory is overwritten.

### Bad Code (Anti-pattern)
```c
void process_username(const char *input) {
    char username[32];
    strcpy(username, input);  // no bounds check -- overflow if input > 31 chars
    log_access(username);
}

void format_path(const char *dir, const char *file) {
    char path[256];
    sprintf(path, "%s/%s", dir, file);  // overflow if combined > 255 chars
    open(path, O_RDONLY);
}

void read_input() {
    char buf[128];
    gets(buf);  // never safe -- no way to limit input length
}
```

### Good Code (Fix)
```c
void process_username(const char *input) {
    char username[32];
    strncpy(username, input, sizeof(username) - 1);
    username[sizeof(username) - 1] = '\0';
    log_access(username);
}

void format_path(const char *dir, const char *file) {
    char path[256];
    int n = snprintf(path, sizeof(path), "%s/%s", dir, file);
    if (n < 0 || (size_t)n >= sizeof(path)) {
        return;  // path was truncated
    }
    open(path, O_RDONLY);
}

void read_input() {
    char buf[128];
    if (fgets(buf, sizeof(buf), stdin) == NULL) {
        return;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes calling `strcpy`, `strcat`, `sprintf`, `gets`, or `scanf` with `%s` format. These functions are inherently unsafe and should be flagged unconditionally. Suggest `strncpy`/`strlcpy`, `strncat`/`strlcat`, `snprintf`, `fgets`, and width-limited `scanf` formats respectively.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  (#match? @func "^(strcpy|strcat|sprintf|gets|vsprintf)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `buffer_overflow`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Integer Overflow in malloc Size Calculation

### Description
Multiplying or adding values to compute a `malloc()` size without overflow checking. If `count * element_size` overflows the `size_t` range, `malloc()` allocates a much smaller buffer than expected, and subsequent writes overflow the heap buffer. This is especially dangerous when `count` comes from untrusted input (file headers, network protocols, user parameters).

### Bad Code (Anti-pattern)
```c
struct record *read_records(FILE *fp) {
    uint32_t count;
    fread(&count, sizeof(count), 1, fp);  // attacker-controlled

    // integer overflow: if count = 0x40000001 and sizeof(struct record) = 16,
    // result wraps to 0x10 (16 bytes), not 1GB+
    struct record *records = malloc(count * sizeof(struct record));
    for (uint32_t i = 0; i < count; i++) {
        fread(&records[i], sizeof(struct record), 1, fp);  // heap overflow
    }
    return records;
}
```

### Good Code (Fix)
```c
struct record *read_records(FILE *fp) {
    uint32_t count;
    fread(&count, sizeof(count), 1, fp);

    if (count > MAX_RECORDS) {
        return NULL;
    }

    // Use calloc which checks for overflow internally
    struct record *records = calloc(count, sizeof(struct record));
    if (!records) {
        return NULL;
    }
    for (uint32_t i = 0; i < count; i++) {
        fread(&records[i], sizeof(struct record), 1, fp);
    }
    return records;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `binary_expression`, `identifier`
- **Detection approach**: Find `call_expression` calling `malloc` where the argument is a `binary_expression` with operator `*` (multiplication). Flag when neither operand is a compile-time constant small enough to make overflow impossible, or when no preceding bounds check limits the variable operand. `calloc(count, size)` is preferred as it checks for overflow internally.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  arguments: (argument_list
    (binary_expression
      left: (_) @lhs
      operator: "*"
      right: (_) @rhs))
  (#eq? @func "malloc"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `integer_overflow`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Use of Uninitialized Memory

### Description
Declaring local variables or allocating memory with `malloc()` without initializing the contents before reading them. In C, local variables and `malloc`-allocated memory contain whatever data was previously at that memory location. Reading uninitialized memory leads to unpredictable behavior and can leak sensitive data (passwords, keys, pointers) from previous stack frames or heap allocations.

### Bad Code (Anti-pattern)
```c
int check_auth(const char *token) {
    int result;  // uninitialized
    if (validate_token(token)) {
        result = 1;
    }
    // if validate_token returns false, result contains stack garbage
    return result;
}

void send_response(int fd) {
    char buffer[512];  // uninitialized -- contains previous stack data
    int len = snprintf(buffer, sizeof(buffer), "OK");
    write(fd, buffer, sizeof(buffer));  // sends 512 bytes, only 2 initialized
}

struct packet *create_packet() {
    struct packet *pkt = malloc(sizeof(struct packet));
    pkt->type = PKT_DATA;
    // pkt->flags, pkt->reserved, pkt->padding are uninitialized
    send_packet(pkt);  // leaks heap data in uninitialized fields
    return pkt;
}
```

### Good Code (Fix)
```c
int check_auth(const char *token) {
    int result = 0;  // explicit initialization
    if (validate_token(token)) {
        result = 1;
    }
    return result;
}

void send_response(int fd) {
    char buffer[512];
    memset(buffer, 0, sizeof(buffer));
    int len = snprintf(buffer, sizeof(buffer), "OK");
    write(fd, buffer, len);  // only send the actual content
}

struct packet *create_packet() {
    struct packet *pkt = calloc(1, sizeof(struct packet));  // zero-initialized
    if (!pkt) return NULL;
    pkt->type = PKT_DATA;
    send_packet(pkt);
    return pkt;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration`, `identifier`, `init_declarator`, `call_expression`
- **Detection approach**: Find `declaration` nodes with no initializer (no `init_declarator` value) for non-pointer local variables in function scope. Also find `call_expression` calling `malloc` (not `calloc`) where the result is used without a subsequent `memset` or field-by-field initialization before being read or passed to another function.
- **S-expression query sketch**:
```scheme
(declaration
  type: (_) @type
  declarator: (identifier) @var_name)
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `uninitialized_memory`
- **Severity**: error
- **Confidence**: medium
