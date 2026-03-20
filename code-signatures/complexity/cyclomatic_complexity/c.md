# Cyclomatic Complexity -- C

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `else if`, `switch` cases, loops (`for`, `while`, `do-while`), logical operators (`&&`, `||`), and ternary expressions (`?:`). High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Each decision point introduces a new execution path that needs its own test case, so high-CC functions demand disproportionate testing effort. In C, where manual memory management and error handling through return codes add implicit branching, elevated CC often correlates with resource leaks and missed error paths. Studies consistently show a strong relationship between cyclomatic complexity and defect density.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c`, `.h`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```c
int process_packet(packet_t *pkt, config_t *cfg)
{
    if (pkt == NULL)
        return -EINVAL;

    if (pkt->version == 1) {
        if (pkt->type == PKT_DATA) {
            if (pkt->length > cfg->max_payload || pkt->flags & FLAG_OVERSIZED) {
                return -EMSGSIZE;
            } else if (pkt->flags & FLAG_COMPRESSED && pkt->flags & FLAG_ENCRYPTED) {
                return handle_compressed_encrypted(pkt);
            } else if (pkt->flags & FLAG_COMPRESSED) {
                return handle_compressed(pkt);
            } else {
                return handle_plain(pkt);
            }
        } else if (pkt->type == PKT_CONTROL) {
            switch (pkt->subtype) {
                case CTRL_KEEPALIVE:
                    return send_ack(pkt);
                case CTRL_RESET:
                    return pkt->flags & FLAG_GRACEFUL ? graceful_reset(pkt) : hard_reset(pkt);
                case CTRL_CONFIG:
                    if (pkt->auth_level >= AUTH_ADMIN) {
                        return apply_config(pkt);
                    }
                    return -EACCES;
                default:
                    return -ENOTSUP;
            }
        } else {
            return -EINVAL;
        }
    } else if (pkt->version == 2) {
        if (pkt->length > cfg->v2_max_payload) {
            return -EMSGSIZE;
        }
        return handle_v2(pkt);
    } else {
        return -EPROTONOSUPPORT;
    }
}
```

### Good Code (Fix)
```c
static int process_v1_data(packet_t *pkt, config_t *cfg)
{
    if (pkt->length > cfg->max_payload || pkt->flags & FLAG_OVERSIZED)
        return -EMSGSIZE;

    if (pkt->flags & FLAG_COMPRESSED && pkt->flags & FLAG_ENCRYPTED)
        return handle_compressed_encrypted(pkt);
    if (pkt->flags & FLAG_COMPRESSED)
        return handle_compressed(pkt);
    return handle_plain(pkt);
}

static int process_v1_control(packet_t *pkt)
{
    switch (pkt->subtype) {
    case CTRL_KEEPALIVE:
        return send_ack(pkt);
    case CTRL_RESET:
        return (pkt->flags & FLAG_GRACEFUL) ? graceful_reset(pkt) : hard_reset(pkt);
    case CTRL_CONFIG:
        return (pkt->auth_level >= AUTH_ADMIN) ? apply_config(pkt) : -EACCES;
    default:
        return -ENOTSUP;
    }
}

static int process_v1(packet_t *pkt, config_t *cfg)
{
    switch (pkt->type) {
    case PKT_DATA:
        return process_v1_data(pkt, cfg);
    case PKT_CONTROL:
        return process_v1_control(pkt);
    default:
        return -EINVAL;
    }
}

static int process_v2(packet_t *pkt, config_t *cfg)
{
    if (pkt->length > cfg->v2_max_payload)
        return -EMSGSIZE;
    return handle_v2(pkt);
}

int process_packet(packet_t *pkt, config_t *cfg)
{
    if (pkt == NULL)
        return -EINVAL;

    switch (pkt->version) {
    case 1:  return process_v1(pkt, cfg);
    case 2:  return process_v2(pkt, cfg);
    default: return -EPROTONOSUPPORT;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_clause`, `case_statement`, `for_statement`, `while_statement`, `do_statement`, `binary_expression` (with `&&`, `||`), `conditional_expression` (`?:`)
- **Detection approach**: Count decision points within a function body. Each `if`, `else if`, `case`, `for`, `while`, `do-while`, `&&`, `||`, and `?:` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_definition body: (compound_statement) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(case_statement) @decision
(for_statement) @decision
(while_statement) @decision
(do_statement) @decision
(conditional_expression) @decision
(binary_expression operator: ["&&" "||"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or switch statements that compound complexity. In C, nested error-handling checks and resource validation often create excessive depth.

### Bad Code (Anti-pattern)
```c
int init_device(device_t *dev, const char *path, config_t *cfg)
{
    if (dev != NULL) {
        if (path != NULL) {
            int fd = open(path, O_RDWR);
            if (fd >= 0) {
                if (ioctl(fd, DEV_GET_INFO, &dev->info) == 0) {
                    dev->buffer = malloc(cfg->buf_size);
                    if (dev->buffer != NULL) {
                        if (configure_device(dev, cfg) == 0) {
                            dev->fd = fd;
                            return 0;
                        } else {
                            free(dev->buffer);
                            close(fd);
                            return -EIO;
                        }
                    } else {
                        close(fd);
                        return -ENOMEM;
                    }
                } else {
                    close(fd);
                    return -EIO;
                }
            } else {
                return -errno;
            }
        } else {
            return -EINVAL;
        }
    } else {
        return -EINVAL;
    }
}
```

### Good Code (Fix)
```c
int init_device(device_t *dev, const char *path, config_t *cfg)
{
    if (dev == NULL || path == NULL)
        return -EINVAL;

    int fd = open(path, O_RDWR);
    if (fd < 0)
        return -errno;

    if (ioctl(fd, DEV_GET_INFO, &dev->info) != 0) {
        close(fd);
        return -EIO;
    }

    dev->buffer = malloc(cfg->buf_size);
    if (dev->buffer == NULL) {
        close(fd);
        return -ENOMEM;
    }

    if (configure_device(dev, cfg) != 0) {
        free(dev->buffer);
        close(fd);
        return -EIO;
    }

    dev->fd = fd;
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `compound_statement` body
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function boundary. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested if statements (3+ levels)
(if_statement
  consequence: (compound_statement
    (if_statement
      consequence: (compound_statement
        (if_statement) @deeply_nested))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
