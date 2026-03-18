# Stringly Typed -- C

## Overview
"Stringly typed" code uses strings where enums, constants, or dedicated types would be more appropriate. String comparisons are fragile — typos compile successfully but fail at runtime.

## Why It's a Tech Debt Concern
Strings bypass type checking. A typo in `"actve"` instead of `"active"` won't be caught at compile time. Refactoring string values requires global search-replace. IDEs can't provide autocompletion for string values.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h

---

## Pattern 1: String Comparisons for State/Status

### Description
Using `strcmp()` string comparisons to determine state transitions, roles, or status instead of `enum` types or `#define` constants.

### Bad Code (Anti-pattern)
```c
#include <string.h>

void process_order(struct Order *order) {
    if (strcmp(order->status, "active") == 0) {
        start_fulfillment(order);
    } else if (strcmp(order->status, "pending") == 0) {
        notify_customer(order);
    } else if (strcmp(order->status, "cancelled") == 0) {
        refund_payment(order);
    } else if (strcmp(order->status, "shipped") == 0) {
        track_delivery(order);
    } else if (strcmp(order->status, "delivered") == 0) {
        request_review(order);
    }
}

const char *get_status_color(const char *status) {
    if (strcmp(status, "active") == 0) return "green";
    if (strcmp(status, "pending") == 0) return "yellow";
    if (strcmp(status, "cancelled") == 0) return "red";
    if (strcmp(status, "shipped") == 0) return "blue";
    if (strcmp(status, "delivered") == 0) return "gray";
    return "white";
}
```

### Good Code (Fix)
```c
enum status {
    STATUS_ACTIVE,
    STATUS_PENDING,
    STATUS_CANCELLED,
    STATUS_SHIPPED,
    STATUS_DELIVERED,
};

void process_order(struct Order *order) {
    switch (order->status) {
    case STATUS_ACTIVE:
        start_fulfillment(order);
        break;
    case STATUS_PENDING:
        notify_customer(order);
        break;
    case STATUS_CANCELLED:
        refund_payment(order);
        break;
    case STATUS_SHIPPED:
        track_delivery(order);
        break;
    case STATUS_DELIVERED:
        request_review(order);
        break;
    }
}

const char *get_status_color(enum status s) {
    switch (s) {
    case STATUS_ACTIVE:    return "green";
    case STATUS_PENDING:   return "yellow";
    case STATUS_CANCELLED: return "red";
    case STATUS_SHIPPED:   return "blue";
    case STATUS_DELIVERED:  return "gray";
    default:               return "white";
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` (`strcmp`), `string_literal`, `if_statement`, `binary_expression`
- **Detection approach**: Find `call_expression` nodes calling `strcmp` (or `strncmp`) where one argument is a string literal, wrapped in a `binary_expression` comparing to `0`. Flag when the same variable is passed to `strcmp` with 3+ different string literals across an `if`/`else if` chain.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (call_expression
    function: (identifier) @func_name
    arguments: (argument_list
      (identifier) @var
      (string_literal) @string_val))
  right: (number_literal))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_status_comparison`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: String Keys for Configuration/Dispatch

### Description
Using string keys for configuration lookup or command dispatch by passing string literals to lookup functions, instead of typed enums or `#define` constants.

### Bad Code (Anti-pattern)
```c
#include <string.h>

const char *get_config(const char *config[], const char *keys[], int count, const char *key) {
    for (int i = 0; i < count; i++) {
        if (strcmp(keys[i], key) == 0) return config[i];
    }
    return NULL;
}

void setup_app(const char *config[], const char *keys[], int count) {
    const char *db_host = get_config(config, keys, count, "database_host");
    const char *db_port = get_config(config, keys, count, "database_port");
    const char *db_name = get_config(config, keys, count, "database_name");
    const char *redis_url = get_config(config, keys, count, "redis_url");
    const char *api_key = get_config(config, keys, count, "api_key");
    const char *log_level = get_config(config, keys, count, "log_level");

    connect_db(db_host, atoi(db_port), db_name);
    connect_redis(redis_url);
    init_logger(log_level);
}

void dispatch_command(const char *cmd, void *data) {
    if (strcmp(cmd, "start") == 0) handle_start(data);
    else if (strcmp(cmd, "stop") == 0) handle_stop(data);
    else if (strcmp(cmd, "pause") == 0) handle_pause(data);
    else if (strcmp(cmd, "resume") == 0) handle_resume(data);
    else if (strcmp(cmd, "reset") == 0) handle_reset(data);
}
```

### Good Code (Fix)
```c
struct app_config {
    const char *db_host;
    int db_port;
    const char *db_name;
    const char *redis_url;
    const char *api_key;
    const char *log_level;
};

void setup_app(const struct app_config *config) {
    connect_db(config->db_host, config->db_port, config->db_name);
    connect_redis(config->redis_url);
    init_logger(config->log_level);
}

enum command {
    CMD_START,
    CMD_STOP,
    CMD_PAUSE,
    CMD_RESUME,
    CMD_RESET,
};

void dispatch_command(enum command cmd, void *data) {
    switch (cmd) {
    case CMD_START:  handle_start(data);  break;
    case CMD_STOP:   handle_stop(data);   break;
    case CMD_PAUSE:  handle_pause(data);  break;
    case CMD_RESUME: handle_resume(data); break;
    case CMD_RESET:  handle_reset(data);  break;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression` with `string_literal` argument, `identifier`
- **Detection approach**: Find repeated calls to the same lookup function where string literals are passed as key arguments. Flag when 5+ different string keys are passed to the same function. Also detect chains of `strcmp` calls dispatching on the same variable with string literals.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  arguments: (argument_list
    (string_literal) @key))
```

### Pipeline Mapping
- **Pipeline name**: `stringly_typed`
- **Pattern name**: `string_key_config`
- **Severity**: info
- **Confidence**: medium
