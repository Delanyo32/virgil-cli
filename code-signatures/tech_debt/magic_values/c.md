# Magic Values -- C

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```c
int process_request(const char *data, size_t len) {
    if (len > 1024) {
        return 413;
    }
    for (int i = 0; i < 3; i++) {
        sleep(86400);
    }
    if (response->status == 200) {
        cache_set(key, data, 3600);
    } else if (response->status == 404) {
        return 0;
    }
    return 0;
}
```

### Good Code (Fix)
```c
#define MAX_PAYLOAD_SIZE 1024
#define MAX_RETRIES 3
#define SECONDS_PER_DAY 86400
#define HTTP_OK 200
#define HTTP_NOT_FOUND 404
#define CACHE_TTL_SECONDS 3600

int process_request(const char *data, size_t len) {
    if (len > MAX_PAYLOAD_SIZE) {
        return HTTP_PAYLOAD_TOO_LARGE;
    }
    for (int i = 0; i < MAX_RETRIES; i++) {
        sleep(SECONDS_PER_DAY);
    }
    if (response->status == HTTP_OK) {
        cache_set(key, data, CACHE_TTL_SECONDS);
    } else if (response->status == HTTP_NOT_FOUND) {
        return 0;
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `number_literal` (excludes 0, 1, -1)
- **Detection approach**: Find `number_literal` nodes in expressions. Exclude literals inside `preproc_def`, `preproc_function_def`, or `enumerator` ancestor nodes, and `declaration` ancestors with a `const` type qualifier. Also exclude `subscript_expression` index positions. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
(number_literal) @number
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```c
int handle_command(const char *cmd) {
    if (strcmp(cmd, "start") == 0) {
        start_service();
    } else if (strcmp(cmd, "stop") == 0) {
        stop_service();
    } else if (strcmp(cmd, "restart") == 0) {
        restart_service();
    }
    const char *mode = getenv("PRODUCTION_MODE");
    if (strcmp(status, "active") == 0) {
        activate();
    }
}
```

### Good Code (Fix)
```c
static const char CMD_START[] = "start";
static const char CMD_STOP[] = "stop";
static const char CMD_RESTART[] = "restart";
static const char STATUS_ACTIVE[] = "active";
static const char ENV_PRODUCTION_MODE[] = "PRODUCTION_MODE";

int handle_command(const char *cmd) {
    if (strcmp(cmd, CMD_START) == 0) {
        start_service();
    } else if (strcmp(cmd, CMD_STOP) == 0) {
        stop_service();
    } else if (strcmp(cmd, CMD_RESTART) == 0) {
        restart_service();
    }
    const char *mode = getenv(ENV_PRODUCTION_MODE);
    if (strcmp(status, STATUS_ACTIVE) == 0) {
        activate();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string_literal` in `argument_list` of `call_expression` (strcmp/strncmp arguments) or direct comparison contexts
- **Detection approach**: Find `string_literal` nodes used as arguments to string comparison functions (`strcmp`, `strncmp`, `strstr`) or environment lookup functions (`getenv`). Exclude format strings (first argument to `printf`/`fprintf`/`sprintf`), logging strings, and header include paths. Flag repeated identical strings across a function or file.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (string_literal) @string_lit))
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
