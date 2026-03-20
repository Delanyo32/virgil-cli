# Concurrency Misuse -- Java

## Overview
Java provides powerful concurrency primitives, but misusing `synchronized` blocks and `Thread.sleep()` in production code leads to performance bottlenecks, unresponsive applications, and subtle concurrency bugs that are difficult to reproduce and diagnose.

## Why It's a Tech Debt Concern
Synchronizing on the wrong object (e.g., a mutable field, a boxed type, or `this` when a private lock is needed) creates either no synchronization at all or contention that spans unrelated code paths. Over-broad synchronized methods serialize all access to an object when only specific fields need protection. `Thread.sleep()` in production code blocks threads, wastes resources, introduces arbitrary delays instead of event-driven scheduling, and cannot be interrupted cleanly.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`
- **Frameworks/libraries**: java.util.concurrent, Spring (async processing), Jakarta EE

---

## Pattern 1: Synchronized on Wrong Object or Too-Broad Scope

### Description
Using `synchronized` on `this` (exposing the lock to external code), on a mutable field (the lock reference can change), on a boxed primitive (Integer/Boolean instances are cached and shared across the JVM), or applying `synchronized` to an entire method when only a few lines need protection. These patterns create either false safety or excessive contention.

### Bad Code (Anti-pattern)
```java
public class UserCache {
    private Map<String, User> cache = new HashMap<>();
    private Boolean isEnabled = true;

    // Synchronized on 'this' — any external code holding a reference
    // to this object can interfere with the lock
    public synchronized User getUser(String id) {
        return cache.get(id);
    }

    // Synchronized on a mutable field — if 'cache' is reassigned,
    // threads synchronize on different objects
    public void updateUser(String id, User user) {
        synchronized (cache) {
            cache.put(id, user);
        }
    }

    // Synchronized on a boxed Boolean — Boolean.TRUE is a JVM-wide
    // singleton, creating contention with unrelated code
    public void toggleEnabled() {
        synchronized (isEnabled) {
            isEnabled = !isEnabled;
        }
    }
}
```

### Good Code (Fix)
```java
public class UserCache {
    private final Object lock = new Object();
    private Map<String, User> cache = new HashMap<>();
    private volatile boolean isEnabled = true;

    public User getUser(String id) {
        synchronized (lock) {
            return cache.get(id);
        }
    }

    public void updateUser(String id, User user) {
        synchronized (lock) {
            cache.put(id, user);
        }
    }

    // For simple boolean toggle, use AtomicBoolean
    private final AtomicBoolean enabled = new AtomicBoolean(true);

    public void toggleEnabled() {
        enabled.compareAndSet(true, false);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `synchronized_statement`, `parenthesized_expression`, `method_declaration` (with `synchronized` modifier)
- **Detection approach**: For `synchronized_statement`, inspect the lock expression: flag `this`, any field access on a mutable (non-final) field, or boxed types (`Boolean`, `Integer`, `Long`, etc.). For `method_declaration` with `synchronized` modifier, flag if the method body has more than 20 lines or contains logic unrelated to the shared state being protected.
- **S-expression query sketch**:
```scheme
(synchronized_statement
  (parenthesized_expression
    (this)) @lock_on_this)

(synchronized_statement
  (parenthesized_expression
    (identifier) @lock_target))

(method_declaration
  (modifiers
    (modifier) @sync_modifier)
  body: (block) @method_body)
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `bad_synchronized_target`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Thread.sleep in Production Code

### Description
Using `Thread.sleep()` for delays, polling, or rate-limiting in production code instead of `ScheduledExecutorService`, `CompletableFuture.delayedExecutor()`, or event-driven mechanisms. `Thread.sleep()` blocks the thread entirely, wastes thread pool capacity, introduces hard-coded timing assumptions, and does not respond to interruption cleanly.

### Bad Code (Anti-pattern)
```java
public class OrderPoller {
    public void pollForUpdates() {
        while (true) {
            List<Order> pending = orderRepository.findPending();
            for (Order order : pending) {
                processOrder(order);
            }
            try {
                Thread.sleep(5000);  // Blocks thread for 5 seconds
            } catch (InterruptedException e) {
                // Swallowed — thread interrupt flag cleared
            }
        }
    }

    public boolean waitForCondition(Supplier<Boolean> condition) {
        for (int i = 0; i < 10; i++) {
            if (condition.get()) return true;
            try {
                Thread.sleep(1000);  // Busy-wait with sleep
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
        return false;
    }
}
```

### Good Code (Fix)
```java
public class OrderPoller {
    private final ScheduledExecutorService scheduler =
        Executors.newSingleThreadScheduledExecutor();

    public void startPolling() {
        scheduler.scheduleWithFixedDelay(this::pollOnce, 0, 5, TimeUnit.SECONDS);
    }

    private void pollOnce() {
        List<Order> pending = orderRepository.findPending();
        for (Order order : pending) {
            processOrder(order);
        }
    }

    public CompletableFuture<Boolean> waitForCondition(Supplier<Boolean> condition) {
        CompletableFuture<Boolean> future = new CompletableFuture<>();
        ScheduledExecutorService executor = Executors.newSingleThreadScheduledExecutor();
        AtomicInteger attempts = new AtomicInteger(0);
        executor.scheduleAtFixedRate(() -> {
            if (condition.get()) {
                future.complete(true);
                executor.shutdown();
            } else if (attempts.incrementAndGet() >= 10) {
                future.complete(false);
                executor.shutdown();
            }
        }, 0, 1, TimeUnit.SECONDS);
        return future;
    }

    public void shutdown() {
        scheduler.shutdown();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `scoped_identifier`, `identifier`
- **Detection approach**: Find `method_invocation` nodes where the method is `sleep` and the object is `Thread` (either `Thread.sleep(...)` or a static import of `sleep`). Exclude test files (`*Test.java`, `*Spec.java`) since `Thread.sleep()` in tests is sometimes acceptable for integration testing.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @class_name
  name: (identifier) @method_name
  arguments: (argument_list
    (integer_literal) @sleep_duration))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `thread_sleep_in_production`
- **Severity**: warning
- **Confidence**: high
