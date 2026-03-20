# Injection -- C

## Overview
Injection vulnerabilities in C arise from the language's low-level string handling and direct system call interfaces. The `system()` and `popen()` functions pass strings to the shell for execution, and `printf`-family functions interpret format specifiers from their arguments. When user-controlled data reaches these functions without validation, attackers gain the ability to execute arbitrary commands or read/write arbitrary memory.

## Why It's a Security Concern
Command injection through `system()` or `popen()` with user-controlled strings allows attackers to execute arbitrary operating system commands with the process's privileges. Format string vulnerabilities allow attackers to read from the stack, write to arbitrary memory addresses, and achieve code execution. Both vulnerability classes are particularly dangerous in C because the language lacks runtime bounds checking and memory safety guarantees.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libc (stdlib.h, stdio.h), POSIX (unistd.h)

---

## Pattern 1: Command Injection via system/popen with User Input

### Description
Passing user-controlled strings to `system()` or `popen()`, which invoke the system shell (`/bin/sh -c`) to execute the command. Shell metacharacters in the input (`;`, `&&`, `|`, backticks, `$()`) allow attackers to chain arbitrary commands.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void ping_host(const char *hostname) {
    char cmd[256];
    snprintf(cmd, sizeof(cmd), "ping -c 4 %s", hostname);
    system(cmd);
}

FILE *list_directory(const char *path) {
    char cmd[512];
    sprintf(cmd, "ls -la %s", path);
    return popen(cmd, "r");
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <ctype.h>

int ping_host(const char *hostname) {
    /* Validate hostname: alphanumeric, dots, and hyphens only */
    for (const char *p = hostname; *p; p++) {
        if (!isalnum(*p) && *p != '.' && *p != '-') {
            fprintf(stderr, "Invalid hostname character: %c\n", *p);
            return -1;
        }
    }

    pid_t pid = fork();
    if (pid == 0) {
        execlp("ping", "ping", "-c", "4", hostname, (char *)NULL);
        _exit(127);
    }
    int status;
    waitpid(pid, &status, 0);
    return WEXITSTATUS(status);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `string_literal`
- **Detection approach**: Find `call_expression` nodes where the function is `system` or `popen` and the argument is a variable (not a string literal) or a variable previously assigned via `sprintf`/`snprintf`/`strcat` with format specifiers (`%s`) incorporating user input. Trace the argument to identify if it includes data from `argv`, `fgets`, `scanf`, `getenv`, or other user-input sources.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @cmd_var))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_system`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Format String Vulnerability

### Description
Passing user-controlled strings directly as the format argument to `printf()`, `fprintf()`, `sprintf()`, `snprintf()`, `syslog()`, or similar functions. Without an explicit format specifier, attackers can use `%x` to read the stack, `%s` to read arbitrary memory, and `%n` to write to arbitrary memory addresses, potentially achieving code execution.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>
#include <syslog.h>

void log_message(const char *user_input) {
    printf(user_input);
}

void log_to_syslog(const char *message) {
    syslog(LOG_INFO, message);
}

void display_error(FILE *fp, const char *error_msg) {
    fprintf(fp, error_msg);
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <syslog.h>

void log_message(const char *user_input) {
    printf("%s", user_input);
}

void log_to_syslog(const char *message) {
    syslog(LOG_INFO, "%s", message);
}

void display_error(FILE *fp, const char *error_msg) {
    fprintf(fp, "%s", error_msg);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes where the function is `printf`, `fprintf`, `sprintf`, `snprintf`, `syslog`, or `dprintf` and the format argument (first for `printf`/`sprintf`/`snprintf`, second for `fprintf`/`syslog`) is an `identifier` or `subscript_expression` (variable) rather than a `string_literal`. When a variable is passed directly as the format string, it indicates the format is controlled externally.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @format_arg))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `format_string_vulnerability`
- **Severity**: error
- **Confidence**: high
