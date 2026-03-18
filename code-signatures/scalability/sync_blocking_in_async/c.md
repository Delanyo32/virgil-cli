# Sync Blocking in Async -- C

## Overview
C does not have built-in async/await, but event-driven programming is common via libraries like libuv (Node.js runtime), libev, and libevent. Blocking calls inside event loop callbacks stall the entire loop, preventing other events from being processed.

## Why It's a Scalability Concern
Event-driven C servers handle thousands of connections on a single thread. A blocking `read()`, `recv()`, or `sleep()` inside a callback freezes all connection handling for the duration of the blocking call, reducing throughput to serial execution.

## Applicability
- **Relevance**: low
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libuv, libev, libevent

---

## Pattern 1: Blocking read()/recv() in Event Loop Callback

### Description
Using blocking system calls like `read()`, `recv()`, `fread()` inside a callback registered with an event loop (libuv, libev, libevent).

### Bad Code (Anti-pattern)
```c
void on_connection(uv_stream_t *server, int status) {
    char buffer[1024];
    int fd = get_client_fd(server);
    ssize_t n = read(fd, buffer, sizeof(buffer)); // blocks event loop
    process_data(buffer, n);
}
```

### Good Code (Fix)
```c
void on_read(uv_stream_t *client, ssize_t nread, const uv_buf_t *buf) {
    if (nread > 0) {
        process_data(buf->base, nread);
    }
    free(buf->base);
}

void on_connection(uv_stream_t *server, int status) {
    uv_tcp_t *client = malloc(sizeof(uv_tcp_t));
    uv_tcp_init(uv_default_loop(), client);
    uv_accept(server, (uv_stream_t *)client);
    uv_read_start((uv_stream_t *)client, alloc_buffer, on_read);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `function_definition`
- **Detection approach**: Find `call_expression` calling `read`, `recv`, `fread`, `fgets` inside a `function_definition` whose name matches common callback patterns or whose parameters include event-loop types (e.g., `uv_stream_t*`, `ev_io*`, `struct event*`). This requires heuristic matching on function signatures.
- **S-expression query sketch**:
```scheme
(function_definition
  declarator: (function_declarator
    parameters: (parameter_list
      (parameter_declaration
        type: (type_identifier) @param_type)))
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_io_in_event_callback`
- **Severity**: warning
- **Confidence**: low

---

## Pattern 2: sleep()/usleep() in Event-Driven Callback

### Description
Using `sleep()`, `usleep()`, or `nanosleep()` inside an event loop callback, which pauses the entire event loop thread.

### Bad Code (Anti-pattern)
```c
void on_timer(uv_timer_t *handle) {
    int result = try_connect();
    if (result < 0) {
        sleep(5); // blocks entire event loop for 5 seconds
        try_connect();
    }
}
```

### Good Code (Fix)
```c
void retry_connect(uv_timer_t *handle) {
    int result = try_connect();
    if (result < 0) {
        // Schedule retry via event loop timer instead of blocking
        uv_timer_start(handle, retry_connect, 5000, 0);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `function_definition`
- **Detection approach**: Find `call_expression` calling `sleep`, `usleep`, `nanosleep` inside a function whose parameters include event-loop callback types.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `sleep_in_event_callback`
- **Severity**: warning
- **Confidence**: low
