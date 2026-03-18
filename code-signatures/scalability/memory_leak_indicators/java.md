# Memory Leak Indicators -- Java

## Overview
Memory leaks in Java occur when objects are unintentionally retained by strong references — unclosed resources, static collections that only grow, ThreadLocal values not removed, or inner classes holding outer references. The GC cannot reclaim these objects despite them being logically unused.

## Why It's a Scalability Concern
Java applications running in containers have fixed heap limits. Leaked objects consume heap space, increasing GC frequency and pause duration. In web servers handling thousands of requests, resource leaks (connections, streams) also exhaust OS-level limits (file descriptors, socket ports), causing cascading failures.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: JDK (collections, threading, I/O), Spring, JDBC
- **Existing pipeline**: `resource_leaks.rs` in `src/audit/pipelines/java/` — extends with additional patterns

---

## Pattern 1: Resource Not in try-with-resources

### Description
Opening a resource (`InputStream`, `Connection`, `Reader`, `Channel`) without using try-with-resources, risking resource leaks if an exception occurs before manual `close()`.

### Bad Code (Anti-pattern)
```java
public String readFile(String path) throws IOException {
    FileInputStream fis = new FileInputStream(path);
    BufferedReader reader = new BufferedReader(new InputStreamReader(fis));
    String content = reader.lines().collect(Collectors.joining("\n"));
    reader.close();
    return content;
}
```

### Good Code (Fix)
```java
public String readFile(String path) throws IOException {
    try (var fis = new FileInputStream(path);
         var reader = new BufferedReader(new InputStreamReader(fis))) {
        return reader.lines().collect(Collectors.joining("\n"));
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_variable_declaration`, `object_creation_expression`, `try_with_resources_statement`
- **Detection approach**: Find `local_variable_declaration` where the value is an `object_creation_expression` of a known `AutoCloseable` type (`FileInputStream`, `BufferedReader`, `Connection`, `Socket`, `Channel`) that is NOT inside a `try_with_resources_statement`'s resource specification.
- **S-expression query sketch**:
```scheme
(local_variable_declaration
  type: (type_identifier) @type
  declarator: (variable_declarator
    value: (object_creation_expression
      type: (type_identifier) @created_type)))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `resource_not_in_twr`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Collection Growth Without Eviction in Loop

### Description
Calling `HashMap.put()`, `ArrayList.add()`, or `HashSet.add()` inside a loop without any corresponding `.remove()`, `.clear()`, or size check, causing unbounded collection growth.

### Bad Code (Anti-pattern)
```java
private final Map<String, UserSession> sessions = new HashMap<>();

public void onUserLogin(String userId, UserSession session) {
    sessions.put(userId, session);
    // never removes expired sessions
}
```

### Good Code (Fix)
```java
private final Map<String, UserSession> sessions = new LinkedHashMap<>() {
    @Override
    protected boolean removeEldestEntry(Map.Entry<String, UserSession> eldest) {
        return size() > 10000 || eldest.getValue().isExpired();
    }
};

public void onUserLogin(String userId, UserSession session) {
    sessions.put(userId, session);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `identifier`, `enhanced_for_statement`
- **Detection approach**: Find `method_invocation` calling `put` or `add` on a collection field, where no `remove`, `clear`, or size check exists in the class. Prioritize fields with `static` modifier or fields in singleton/service classes.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @collection
  name: (identifier) @method
  (#match? @method "^(put|add)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `collection_growth_no_eviction`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: Static Collection With Add But No Remove

### Description
A `static` field of type `List`, `Map`, or `Set` that has `.add()` or `.put()` calls but no `.remove()`, `.clear()`, or eviction anywhere in the class. Static fields persist for the lifetime of the class loader.

### Bad Code (Anti-pattern)
```java
public class MetricsCollector {
    private static final List<Metric> metrics = new ArrayList<>();

    public static void record(String name, double value) {
        metrics.add(new Metric(name, value, Instant.now()));
    }
}
```

### Good Code (Fix)
```java
public class MetricsCollector {
    private static final Deque<Metric> metrics = new ArrayDeque<>();
    private static final int MAX_METRICS = 10000;

    public static synchronized void record(String name, double value) {
        metrics.add(new Metric(name, value, Instant.now()));
        while (metrics.size() > MAX_METRICS) {
            metrics.removeFirst();
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration`, `modifiers`, `method_invocation`
- **Detection approach**: Find `field_declaration` with `static` modifier and a collection type. Search the class for `method_invocation` calling `.add()`, `.put()` on that field. Flag if no `.remove()`, `.clear()`, `.poll()`, `.removeFirst()` exists on the same field.
- **S-expression query sketch**:
```scheme
(field_declaration
  (modifiers "static")
  type: (generic_type
    (type_identifier) @collection_type)
  declarator: (variable_declarator
    name: (identifier) @field_name))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `static_collection_growth`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 4: ThreadLocal Without .remove()

### Description
Setting `ThreadLocal` values via `.set()` without calling `.remove()` in a `finally` block, causing the value to persist as long as the thread lives (which in thread pools can be the entire application lifetime).

### Bad Code (Anti-pattern)
```java
private static final ThreadLocal<UserContext> userContext = new ThreadLocal<>();

public void handleRequest(HttpServletRequest req) {
    userContext.set(new UserContext(req.getUserPrincipal()));
    processRequest(req);
    // forgot to remove — leaks if thread is reused from pool
}
```

### Good Code (Fix)
```java
private static final ThreadLocal<UserContext> userContext = new ThreadLocal<>();

public void handleRequest(HttpServletRequest req) {
    try {
        userContext.set(new UserContext(req.getUserPrincipal()));
        processRequest(req);
    } finally {
        userContext.remove();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `identifier`, `try_statement`
- **Detection approach**: Find `method_invocation` calling `.set()` on a `ThreadLocal` variable. Check the enclosing method for a `finally` block containing `.remove()` on the same variable. Flag if no `.remove()` exists.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @tl_var
  name: (identifier) @method
  (#eq? @method "set"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `threadlocal_no_remove`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: Inner Class Holding Outer Reference

### Description
Non-static inner classes implicitly hold a reference to their enclosing instance. When the inner class instance outlives the outer instance (e.g., passed to a callback, stored in a collection), the outer instance cannot be garbage collected.

### Bad Code (Anti-pattern)
```java
public class Activity {
    private final byte[] largeData = new byte[10_000_000];

    public Runnable createTask() {
        return new Runnable() {
            @Override
            public void run() {
                // implicitly holds reference to Activity.this (and largeData)
                System.out.println("running task");
            }
        };
    }
}
```

### Good Code (Fix)
```java
public class Activity {
    private final byte[] largeData = new byte[10_000_000];

    public Runnable createTask() {
        return () -> {
            System.out.println("running task");
        };
    }
    // Or use a static inner class if state is needed:
    private static class Task implements Runnable {
        @Override
        public void run() {
            System.out.println("running task");
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `class_body`, `return_statement`
- **Detection approach**: Find `object_creation_expression` that creates an anonymous class (has a `class_body`) inside a `return_statement` or assigned to a field/collection. The anonymous class implicitly references the outer `this`. Flag if the anonymous class is returned or stored outside the method scope.
- **S-expression query sketch**:
```scheme
(return_statement
  (object_creation_expression
    type: (type_identifier) @type
    (class_body)))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `inner_class_outer_ref`
- **Severity**: info
- **Confidence**: low
