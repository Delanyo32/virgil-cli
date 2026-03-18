# Race Conditions -- C

## Overview
C programs are vulnerable to race conditions through two primary vectors: TOCTOU (time-of-check-to-time-of-use) vulnerabilities where `access()` or `stat()` is called before `open()`, and data races where shared variables in multi-threaded programs (pthreads) are accessed without mutex protection. Both are pervasive in systems programming and have been the root cause of numerous privilege escalation vulnerabilities in Unix/Linux system utilities and daemons.

## Why It's a Security Concern
TOCTOU races in C are a classic privilege escalation vector -- setuid programs that call `access()` to check permissions with the real UID, then `open()` with the effective UID, can be exploited by swapping the target file with a symlink between the two calls. Data races in pthreads programs lead to corrupted state, undefined behavior, and memory safety violations that can be exploited for arbitrary code execution, especially in network daemons handling concurrent connections.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: POSIX (unistd.h, fcntl.h, pthread.h), libc

---

## Pattern 1: TOCTOU via access() then open()

### Description
Using `access()` to check whether the calling process has permission to open a file, then calling `open()` based on the result. The `access()` function checks permissions using the real UID/GID (not the effective UID/GID), so setuid programs use it to verify the caller's actual permissions. However, between the `access()` check and the `open()` call, an attacker can replace the checked file with a symlink to a privileged file, causing `open()` to operate on the wrong target.

### Bad Code (Anti-pattern)
```c
#include <unistd.h>
#include <fcntl.h>
#include <stdio.h>

void read_user_file(const char *path) {
    // Check if real user has read permission
    if (access(path, R_OK) == 0) {
        // RACE: attacker replaces path with symlink to /etc/shadow
        int fd = open(path, O_RDONLY);
        if (fd >= 0) {
            char buf[4096];
            ssize_t n = read(fd, buf, sizeof(buf));
            write(STDOUT_FILENO, buf, n);
            close(fd);
        }
    } else {
        fprintf(stderr, "Permission denied\n");
    }
}
```

### Good Code (Fix)
```c
#include <unistd.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>

void read_user_file(const char *path) {
    // Open first, then check -- eliminates the race window
    int fd = open(path, O_RDONLY | O_NOFOLLOW);  // O_NOFOLLOW prevents symlink traversal
    if (fd < 0) {
        perror("open");
        return;
    }

    // Verify permissions on the opened file descriptor using fstat
    struct stat st;
    if (fstat(fd, &st) < 0 || (st.st_mode & S_IROTH) == 0) {
        fprintf(stderr, "Permission denied\n");
        close(fd);
        return;
    }

    char buf[4096];
    ssize_t n = read(fd, buf, sizeof(buf));
    if (n > 0) write(STDOUT_FILENO, buf, n);
    close(fd);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `if_statement`, `argument_list`
- **Detection approach**: Find `if_statement` nodes whose condition contains a `call_expression` calling `access()` with a path variable, where the body contains a `call_expression` calling `open()`, `fopen()`, or `creat()` with the same path variable. Also detect `stat()` or `lstat()` followed by `open()` on the same path. The two-step pattern with the same path argument is the vulnerability indicator.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (binary_expression
    left: (call_expression
      function: (identifier) @check_func
      (#eq? @check_func "access")
      arguments: (argument_list
        (identifier) @path_var)))
  consequence: (compound_statement
    (declaration
      declarator: (init_declarator
        value: (call_expression
          function: (identifier) @open_func
          (#eq? @open_func "open"))))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `access_then_open`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Data Race in pthreads Without Mutex

### Description
Accessing a shared global or heap-allocated variable from multiple pthreads without protecting the access with `pthread_mutex_lock()`/`pthread_mutex_unlock()` or other synchronization primitives. Since C provides no memory model guarantees without explicit synchronization, concurrent reads and writes produce undefined behavior -- not just incorrect values but potentially exploitable memory corruption.

### Bad Code (Anti-pattern)
```c
#include <pthread.h>

int balance = 1000;

void *withdraw(void *arg) {
    int amount = *(int *)arg;
    // DATA RACE: read-check-modify without mutex
    if (balance >= amount) {
        balance -= amount;  // concurrent withdrawals can overdraw
    }
    return NULL;
}

int main() {
    pthread_t t1, t2;
    int amt = 800;
    pthread_create(&t1, NULL, withdraw, &amt);
    pthread_create(&t2, NULL, withdraw, &amt);
    pthread_join(t1, NULL);
    pthread_join(t2, NULL);
    return 0;
}
```

### Good Code (Fix)
```c
#include <pthread.h>

int balance = 1000;
pthread_mutex_t balance_lock = PTHREAD_MUTEX_INITIALIZER;

void *withdraw(void *arg) {
    int amount = *(int *)arg;
    pthread_mutex_lock(&balance_lock);
    if (balance >= amount) {
        balance -= amount;
    }
    pthread_mutex_unlock(&balance_lock);
    return NULL;
}

int main() {
    pthread_t t1, t2;
    int amt = 800;
    pthread_create(&t1, NULL, withdraw, &amt);
    pthread_create(&t2, NULL, withdraw, &amt);
    pthread_join(t1, NULL);
    pthread_join(t2, NULL);
    pthread_mutex_destroy(&balance_lock);
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `function_definition`, `compound_assignment_expression`, `if_statement`
- **Detection approach**: Find functions that are passed as the third argument to `pthread_create()` (the thread start routine), and check whether those functions access global variables (declared outside any function) with compound assignment (`+=`, `-=`) or direct assignment without an enclosing `pthread_mutex_lock()`/`pthread_mutex_unlock()` pair. The combination of pthreads usage + global variable mutation + no mutex is the indicator.
- **S-expression query sketch**:
```scheme
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func_name)
  body: (compound_statement
    (expression_statement
      (assignment_expression
        left: (identifier) @shared_var
        right: (binary_expression)))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `pthread_data_race`
- **Severity**: error
- **Confidence**: high
