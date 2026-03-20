# Insecure Deserialization -- C

## Overview
Insecure deserialization in C manifests as reading binary data from untrusted sources into structs or buffers without validating sizes, offsets, or field values. Since C has no built-in serialization framework, developers implement custom binary protocols that are prone to buffer overflows, integer overflows, and out-of-bounds reads when parsing crafted input.

## Why It's a Security Concern
C programs that deserialize binary data typically use `memcpy()`, `fread()`, or direct pointer casting to populate structs from raw bytes. Without bounds checking, an attacker can supply a crafted binary payload with oversized length fields, negative offsets, or malformed headers that cause buffer overflows, heap corruption, or information disclosure. These are classic memory corruption vulnerabilities that can lead to arbitrary code execution.

## Applicability
- **Relevance**: medium
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libc (fread, memcpy, read), custom binary protocols, network daemons, file parsers

---

## Pattern 1: Deserializing Binary Data Without Bounds Checking

### Description
Reading a length or count field from untrusted binary input and using it directly in `memcpy()`, `malloc()`, `fread()`, or array indexing without validating that the value is within expected bounds. This allows attackers to trigger buffer overflows or allocate excessive memory via crafted input.

### Bad Code (Anti-pattern)
```c
struct message {
    uint32_t length;
    char data[];
};

int process_message(int fd) {
    uint32_t length;
    read(fd, &length, sizeof(length));      // Attacker controls length
    char *buffer = malloc(length);           // No upper bound check — possible OOM
    read(fd, buffer, length);               // Buffer overflow if allocation failed silently
    struct record *rec = (struct record *)buffer;  // Type punning without validation
    process_record(rec);
    free(buffer);
    return 0;
}
```

### Good Code (Fix)
```c
#define MAX_MESSAGE_SIZE (1024 * 1024)  // 1 MB limit

int process_message(int fd) {
    uint32_t length;
    if (read(fd, &length, sizeof(length)) != sizeof(length)) {
        return -1;
    }
    length = ntohl(length);  // Handle endianness
    if (length > MAX_MESSAGE_SIZE || length < sizeof(struct record)) {
        return -1;  // Reject oversized or undersized messages
    }
    char *buffer = malloc(length);
    if (!buffer) {
        return -1;
    }
    ssize_t total = 0;
    while (total < (ssize_t)length) {
        ssize_t n = read(fd, buffer + total, length - total);
        if (n <= 0) { free(buffer); return -1; }
        total += n;
    }
    struct record *rec = (struct record *)buffer;
    if (!validate_record(rec, length)) {  // Validate internal fields
        free(buffer);
        return -1;
    }
    process_record(rec);
    free(buffer);
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `cast_expression`, `pointer_expression`
- **Detection approach**: Find patterns where `read()`, `fread()`, or `recv()` writes into a buffer, followed by `memcpy()` or pointer casting (`(struct_type *)buffer`) without an intervening bounds check (comparison with a maximum constant). Track the variable holding the length value and check if it is validated before use in `malloc()` or `memcpy()`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (_) @buffer
    (_) @size))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `binary_no_bounds_check`
- **Severity**: error
- **Confidence**: medium
