# Race Conditions -- Java

## Overview
Java's built-in threading support makes concurrent programming common, but shared mutable state accessed from multiple threads without proper synchronization leads to data races. Two prevalent patterns are check-then-act sequences without synchronization (e.g., lazy initialization, conditional updates) and non-atomic compound operations on shared state (e.g., read-modify-write on fields not protected by synchronized blocks or atomic classes).

## Why It's a Security Concern
Race conditions in Java can lead to corrupted data structures, broken security invariants, double-processing of financial transactions, authentication bypasses through stale permission caches, and denial of service via infinite loops in corrupted `HashMap` structures (a classic Java concurrency bug). In enterprise applications handling concurrent requests, these races are reliably exploitable under load.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.lang.Thread, java.util.concurrent, Spring, Jakarta EE, servlet containers

---

## Pattern 1: Check-then-Act Without Synchronization

### Description
Reading a shared variable to check a condition and then acting on it (assigning, returning, initializing) without holding a lock across both operations. The classic example is lazy initialization where a null check and assignment are not synchronized, allowing multiple threads to create duplicate instances or observe partially constructed objects.

### Bad Code (Anti-pattern)
```java
public class ConnectionPool {
    private static ConnectionPool instance;

    public static ConnectionPool getInstance() {
        // RACE: two threads can both see instance == null
        if (instance == null) {
            instance = new ConnectionPool();  // may be created twice
        }
        return instance;
    }
}
```

### Good Code (Fix)
```java
public class ConnectionPool {
    private static volatile ConnectionPool instance;

    public static ConnectionPool getInstance() {
        ConnectionPool local = instance;
        if (local == null) {
            synchronized (ConnectionPool.class) {
                local = instance;
                if (local == null) {
                    instance = local = new ConnectionPool();
                }
            }
        }
        return local;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `binary_expression`, `assignment_expression`, `field_access`, `null_literal`
- **Detection approach**: Find `if_statement` nodes where the condition is a `binary_expression` comparing a field (non-local variable) to `null`, and the body contains an `assignment_expression` assigning to that same field. Flag when the `if_statement` is not inside a `synchronized_statement` and the field is not declared `volatile`. This detects the classic unsynchronized lazy initialization anti-pattern.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (binary_expression
    left: (identifier) @field
    right: (null_literal))
  consequence: (block
    (expression_statement
      (assignment_expression
        left: (identifier) @assigned_field))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `check_then_act`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Non-Atomic Compound Operations on Shared State

### Description
Performing read-modify-write operations (increment, decrement, toggle) on shared fields without synchronization or atomic classes. Operations like `counter++` compile to a read, an add, and a write -- three separate bytecode instructions between which other threads can interleave, causing lost updates. Similarly, `map.put(key, map.get(key) + 1)` is a compound operation that is not atomic.

### Bad Code (Anti-pattern)
```java
public class RequestCounter {
    private int count = 0;

    // Called from multiple servlet threads concurrently
    public void recordRequest() {
        count++;  // NOT atomic: read + increment + write
    }

    public int getCount() {
        return count;  // may read stale value without volatile/synchronization
    }
}
```

### Good Code (Fix)
```java
import java.util.concurrent.atomic.AtomicInteger;

public class RequestCounter {
    private final AtomicInteger count = new AtomicInteger(0);

    public void recordRequest() {
        count.incrementAndGet();  // atomic CAS operation
    }

    public int getCount() {
        return count.get();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `update_expression`, `field_declaration`, `method_declaration`, `assignment_expression`
- **Detection approach**: Find `update_expression` nodes (e.g., `count++`, `--count`) or `assignment_expression` nodes with compound operators (`+=`, `-=`) where the target is a field (declared at class level, not local). Flag when the enclosing `method_declaration` is not `synchronized` and the expression is not inside a `synchronized_statement`, and the field is not of an `Atomic*` type.
- **S-expression query sketch**:
```scheme
(method_declaration
  body: (block
    (expression_statement
      (update_expression
        (identifier) @field))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `non_atomic_compound_op`
- **Severity**: error
- **Confidence**: high
