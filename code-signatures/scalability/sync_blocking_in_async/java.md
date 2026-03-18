# Sync Blocking in Async -- Java

## Overview
Synchronous blocking in Java async contexts occurs when blocking calls like `Thread.sleep()`, blocking I/O, or `.get()` on `CompletableFuture` are used inside `CompletableFuture` chains, virtual threads, or reactive pipelines, stalling the shared thread pool.

## Why It's a Scalability Concern
`CompletableFuture.supplyAsync()` uses the `ForkJoinPool.commonPool()` by default, which has a limited number of threads (typically CPU count - 1). Blocking one of these threads reduces the pool's capacity. With enough blocking calls, the common pool is exhausted and no other async tasks can execute, causing system-wide slowdowns.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: CompletableFuture, ForkJoinPool, Virtual Threads (Project Loom), Spring WebFlux

---

## Pattern 1: Thread.sleep() in CompletableFuture Lambda

### Description
Using `Thread.sleep()` inside a `CompletableFuture.supplyAsync()` or `.thenApplyAsync()` lambda, blocking the shared fork-join pool thread.

### Bad Code (Anti-pattern)
```java
public CompletableFuture<String> fetchWithRetry(String url) {
    return CompletableFuture.supplyAsync(() -> {
        for (int i = 0; i < 3; i++) {
            try {
                return httpClient.send(request, BodyHandlers.ofString()).body();
            } catch (Exception e) {
                try { Thread.sleep(1000 * (i + 1)); } catch (InterruptedException ie) { break; }
            }
        }
        throw new RuntimeException("All retries failed");
    });
}
```

### Good Code (Fix)
```java
public CompletableFuture<String> fetchWithRetry(String url) {
    return CompletableFuture.supplyAsync(() -> {
        return httpClient.sendAsync(request, BodyHandlers.ofString())
            .thenApply(HttpResponse::body);
    }).thenCompose(Function.identity())
    .exceptionallyCompose(ex -> {
        return CompletableFuture.delayedExecutor(1, TimeUnit.SECONDS)
            .execute(() -> {});
        // Use scheduled executor for retry delay
    });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `lambda_expression`, `identifier`
- **Detection approach**: Find `method_invocation` calling `Thread.sleep` inside a `lambda_expression` that is an argument to `supplyAsync`, `thenApplyAsync`, `thenRunAsync`, or similar async methods.
- **S-expression query sketch**:
```scheme
(lambda_expression
  body: (block
    (expression_statement
      (method_invocation
        object: (identifier) @class
        name: (identifier) @method))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `thread_sleep_in_async_lambda`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Blocking InputStream.read() in CompletableFuture

### Description
Using blocking `InputStream.read()`, `BufferedReader.readLine()`, or `Scanner.nextLine()` inside a `CompletableFuture` chain.

### Bad Code (Anti-pattern)
```java
public CompletableFuture<byte[]> processFile(String path) {
    return CompletableFuture.supplyAsync(() -> {
        try (InputStream is = new FileInputStream(path)) {
            return is.readAllBytes();
        } catch (IOException e) {
            throw new CompletionException(e);
        }
    });
}
```

### Good Code (Fix)
```java
public CompletableFuture<byte[]> processFile(String path) {
    return CompletableFuture.supplyAsync(() -> {
        try {
            return Files.readAllBytes(Path.of(path));
        } catch (IOException e) {
            throw new CompletionException(e);
        }
    }, Executors.newVirtualThreadPerTaskExecutor());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `lambda_expression`
- **Detection approach**: Find `method_invocation` calling `read`, `readAllBytes`, `readLine` on stream-like objects inside `lambda_expression` arguments to `supplyAsync` or `thenApplyAsync`.
- **S-expression query sketch**:
```scheme
(lambda_expression
  body: (block
    (local_variable_declaration
      declarator: (variable_declarator
        value: (method_invocation
          name: (identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_io_in_async_lambda`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: .get() Blocking on CompletableFuture

### Description
Calling `.get()` on a `CompletableFuture` inside another async context, which synchronously blocks the calling thread until the future completes.

### Bad Code (Anti-pattern)
```java
public CompletableFuture<OrderDto> getOrderWithCustomer(Long orderId) {
    return CompletableFuture.supplyAsync(() -> {
        Order order = orderRepo.findById(orderId).orElseThrow();
        Customer customer = customerService.getCustomerAsync(order.getCustomerId()).get();
        return new OrderDto(order, customer);
    });
}
```

### Good Code (Fix)
```java
public CompletableFuture<OrderDto> getOrderWithCustomer(Long orderId) {
    return CompletableFuture.supplyAsync(() -> orderRepo.findById(orderId).orElseThrow())
        .thenCompose(order -> customerService.getCustomerAsync(order.getCustomerId())
            .thenApply(customer -> new OrderDto(order, customer)));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `identifier`
- **Detection approach**: Find `method_invocation` calling `.get()` with zero arguments on a `CompletableFuture`-typed expression (look for variables assigned from methods returning `CompletableFuture` or ending in `Async`).
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (method_invocation) @future_call
  name: (identifier) @method
  arguments: (argument_list)
  (#eq? @method "get"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `future_get_blocking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 4: JDBC in Virtual Threads Without Pooling

### Description
Using JDBC calls directly in virtual threads (Project Loom) without connection pooling, which can exhaust database connections since virtual threads are cheap to create but database connections are not.

### Bad Code (Anti-pattern)
```java
public void processAll(List<Integer> ids) {
    try (var executor = Executors.newVirtualThreadPerTaskExecutor()) {
        for (int id : ids) {
            executor.submit(() -> {
                Connection conn = DriverManager.getConnection(DB_URL);
                PreparedStatement ps = conn.prepareStatement("SELECT * FROM users WHERE id = ?");
                ps.setInt(1, id);
                ResultSet rs = ps.executeQuery();
                process(rs);
                conn.close();
            });
        }
    }
}
```

### Good Code (Fix)
```java
public void processAll(List<Integer> ids, DataSource dataSource) {
    try (var executor = Executors.newVirtualThreadPerTaskExecutor()) {
        for (int id : ids) {
            executor.submit(() -> {
                try (Connection conn = dataSource.getConnection();
                     PreparedStatement ps = conn.prepareStatement("SELECT * FROM users WHERE id = ?")) {
                    ps.setInt(1, id);
                    try (ResultSet rs = ps.executeQuery()) {
                        process(rs);
                    }
                }
            });
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `lambda_expression`
- **Detection approach**: Find `method_invocation` calling `DriverManager.getConnection` inside a `lambda_expression` argument to `.submit()` on a virtual thread executor. Look for `newVirtualThreadPerTaskExecutor` in the containing scope.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @class
  name: (identifier) @method
  (#eq? @class "DriverManager")
  (#eq? @method "getConnection"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `jdbc_in_virtual_thread`
- **Severity**: warning
- **Confidence**: medium
