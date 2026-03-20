# Path Traversal -- C

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the system by crafting paths that escape the intended base directory. In C programs, this is especially dangerous because they often run with elevated privileges (daemons, setuid binaries) and lack runtime sandboxing.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: POSIX (stdlib.h, stdio.h, unistd.h), libc

---

## Pattern 1: User Input in File Path

### Description
Using `snprintf()` or `sprintf()` to construct a file path by combining a base directory with user-supplied input without resolving the canonical path via `realpath()` and verifying it starts with the intended base directory.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>

void serve_file(const char *base, const char *user_input) {
    char path[PATH_MAX];
    snprintf(path, sizeof(path), "%s/%s", base, user_input);
    FILE *f = fopen(path, "r");
    if (f) {
        char buf[4096];
        while (fgets(buf, sizeof(buf), f)) {
            fputs(buf, stdout);
        }
        fclose(f);
    }
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

int serve_file(const char *base, const char *user_input) {
    char constructed[PATH_MAX];
    snprintf(constructed, sizeof(constructed), "%s/%s", base, user_input);

    char resolved_base[PATH_MAX];
    char resolved_path[PATH_MAX];
    if (!realpath(base, resolved_base) || !realpath(constructed, resolved_path)) {
        return -1;
    }
    if (strncmp(resolved_path, resolved_base, strlen(resolved_base)) != 0) {
        return -1; /* path escapes base directory */
    }

    FILE *f = fopen(resolved_path, "r");
    if (f) {
        char buf[4096];
        while (fgets(buf, sizeof(buf), f)) {
            fputs(buf, stdout);
        }
        fclose(f);
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `string_literal`, `argument_list`
- **Detection approach**: Find `call_expression` nodes invoking `snprintf` or `sprintf` where the format string contains a path pattern like `"%s/%s"` and one argument is user-supplied (function parameter). Flag when the resulting buffer is passed to `fopen()`, `open()`, `stat()`, or similar without a preceding `realpath()` + `strncmp()` check.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (_)
    (_)
    (string_literal) @format_str
    (_)
    (identifier) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `user_input_in_file_path`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Directory Traversal via ../

### Description
Accepting file paths that contain `../` sequences without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>

void read_upload(const char *filename) {
    /* No check for ".." — attacker sends "../../etc/passwd" */
    char path[256];
    snprintf(path, sizeof(path), "./uploads/%s", filename);
    FILE *f = fopen(path, "r");
    if (f) {
        char buf[4096];
        while (fgets(buf, sizeof(buf), f)) {
            fputs(buf, stdout);
        }
        fclose(f);
    }
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

int read_upload(const char *filename) {
    if (strstr(filename, "..") != NULL) {
        fprintf(stderr, "Invalid filename\n");
        return -1;
    }

    char constructed[PATH_MAX];
    snprintf(constructed, sizeof(constructed), "./uploads/%s", filename);

    char resolved_base[PATH_MAX];
    char resolved_path[PATH_MAX];
    if (!realpath("./uploads", resolved_base) || !realpath(constructed, resolved_path)) {
        return -1;
    }
    if (strncmp(resolved_path, resolved_base, strlen(resolved_base)) != 0) {
        return -1;
    }

    FILE *f = fopen(resolved_path, "r");
    if (f) {
        char buf[4096];
        while (fgets(buf, sizeof(buf), f)) {
            fputs(buf, stdout);
        }
        fclose(f);
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `string_literal`
- **Detection approach**: Find `call_expression` nodes invoking `fopen`, `open`, `stat`, or similar, where the path argument is a buffer previously filled by `snprintf`/`sprintf` with a path pattern and user input. Flag when there is no preceding check for `".."` via `strstr(filename, "..")` or `realpath()` + `strncmp()` validation.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @path_buf
    (string_literal) @mode))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
