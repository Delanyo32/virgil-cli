# Resource Exhaustion -- C

## Overview
Resource exhaustion vulnerabilities in C arise from unbounded memory allocation using user-controlled size values passed to `malloc()`, `calloc()`, or `realloc()`, and from unbounded process creation via `fork()` in loops without proper limits. C provides no runtime bounds checking, making these patterns especially dangerous -- a single malicious input can consume all system memory or all available process slots.

## Why It's a Security Concern
Unbounded `malloc(user_size)` allows attackers to request gigabytes of memory, causing the system to swap heavily or the OOM killer to terminate critical processes. Even if `malloc` returns `NULL`, many programs fail to check and proceed to dereference the null pointer, causing crashes. Unbounded `fork()` (fork bombs) can exhaust the OS process table, rendering the entire system unresponsive and requiring a reboot. Both attacks can be triggered by a single malicious request or input.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libc (malloc, calloc, realloc, fork), POSIX

---

## Pattern 1: Unbounded Allocation -- malloc(user_controlled_size)

### Description
Passing a value derived from user input (network packet fields, command-line arguments, file headers, protocol length fields) directly to `malloc()`, `calloc()`, or `realloc()` without validating it against a maximum bound. Attackers can supply extremely large values to exhaust available memory.

### Bad Code (Anti-pattern)
```c
#include <stdlib.h>
#include <string.h>

struct packet_header {
    uint32_t payload_length;
    uint16_t type;
};

void process_packet(int sockfd) {
    struct packet_header header;
    recv(sockfd, &header, sizeof(header), 0);

    // User-controlled length used directly for allocation
    char *payload = malloc(header.payload_length);
    if (payload == NULL) return;

    recv(sockfd, payload, header.payload_length, 0);
    handle_payload(payload, header.payload_length);
    free(payload);
}

void read_records(FILE *fp) {
    uint32_t count;
    fread(&count, sizeof(count), 1, fp);

    // User-controlled count determines allocation
    struct record *records = calloc(count, sizeof(struct record));
    if (!records) return;

    fread(records, sizeof(struct record), count, fp);
    process_records(records, count);
    free(records);
}
```

### Good Code (Fix)
```c
#include <stdlib.h>
#include <string.h>

#define MAX_PAYLOAD_SIZE (10 * 1024 * 1024)  /* 10 MB */
#define MAX_RECORD_COUNT 100000

struct packet_header {
    uint32_t payload_length;
    uint16_t type;
};

void process_packet(int sockfd) {
    struct packet_header header;
    if (recv(sockfd, &header, sizeof(header), 0) != sizeof(header))
        return;

    // Validate size before allocation
    if (header.payload_length == 0 || header.payload_length > MAX_PAYLOAD_SIZE)
        return;

    char *payload = malloc(header.payload_length);
    if (payload == NULL) return;

    ssize_t received = recv(sockfd, payload, header.payload_length, 0);
    if (received > 0)
        handle_payload(payload, (size_t)received);
    free(payload);
}

void read_records(FILE *fp) {
    uint32_t count;
    if (fread(&count, sizeof(count), 1, fp) != 1)
        return;

    // Cap record count
    if (count > MAX_RECORD_COUNT)
        return;

    struct record *records = calloc(count, sizeof(struct record));
    if (!records) return;

    size_t read = fread(records, sizeof(struct record), count, fp);
    process_records(records, read);
    free(records);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `field_expression`
- **Detection approach**: Find `call_expression` nodes invoking `malloc`, `calloc`, or `realloc` where the size argument is an `identifier` or `field_expression` (struct field access like `header.length`) rather than a constant or `sizeof` expression. Check the enclosing function for a preceding comparison of that variable against a constant maximum. Flag when no bounds check is found.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (field_expression
      field: (field_identifier) @size_field)))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_malloc`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Fork Bomb / Unbounded Process Creation

### Description
Calling `fork()` inside a loop where the iteration count is derived from user input, or failing to limit the number of child processes created. Without bounds, an attacker can cause exponential process creation (fork bomb) or linear but unbounded process spawning, exhausting the system's process table and rendering the entire machine unresponsive.

### Bad Code (Anti-pattern)
```c
#include <unistd.h>
#include <stdlib.h>
#include <sys/wait.h>

void handle_requests(int request_count) {
    // User-controlled count with no limit
    for (int i = 0; i < request_count; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            process_request(i);
            exit(0);
        }
        // No wait -- children accumulate
    }
}

void spawn_workers(const char *count_str) {
    int count = atoi(count_str);  // From user input
    for (int i = 0; i < count; i++) {
        if (fork() == 0) {
            do_work();
            exit(0);
        }
    }
}
```

### Good Code (Fix)
```c
#include <unistd.h>
#include <stdlib.h>
#include <sys/wait.h>

#define MAX_CHILDREN 64

void handle_requests(int request_count) {
    // Enforce upper bound
    if (request_count > MAX_CHILDREN)
        request_count = MAX_CHILDREN;

    int active = 0;
    for (int i = 0; i < request_count; i++) {
        pid_t pid = fork();
        if (pid < 0) {
            perror("fork");
            break;
        }
        if (pid == 0) {
            process_request(i);
            exit(0);
        }
        active++;
        // Limit concurrent children
        if (active >= MAX_CHILDREN) {
            wait(NULL);
            active--;
        }
    }
    // Reap remaining children
    while (active > 0) {
        wait(NULL);
        active--;
    }
}

void spawn_workers(const char *count_str) {
    int count = atoi(count_str);
    if (count <= 0 || count > MAX_CHILDREN)
        count = MAX_CHILDREN;

    for (int i = 0; i < count; i++) {
        pid_t pid = fork();
        if (pid < 0) break;
        if (pid == 0) {
            do_work();
            exit(0);
        }
    }
    // Wait for all children
    while (wait(NULL) > 0);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `for_statement`, `identifier`, `binary_expression`
- **Detection approach**: Find `call_expression` nodes invoking `fork()` inside `for_statement` or `while_statement` loops. Check if the loop bound is a variable (not a compile-time constant) and whether there is a preceding comparison capping that variable against `MAX_CHILDREN` or similar constant. Also check for the absence of `wait()` or `waitpid()` calls inside the loop body that would limit concurrent child processes.
- **S-expression query sketch**:
```scheme
(for_statement
  condition: (binary_expression
    right: (identifier) @bound)
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_fork`
- **Severity**: warning
- **Confidence**: medium
