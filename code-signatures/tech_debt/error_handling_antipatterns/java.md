# Error Handling Anti-patterns -- Java

## Overview
Errors that are silently swallowed, broadly caught, or used for control flow make debugging impossible and hide real failures. In Java, empty catch blocks, catching `Exception` or `Throwable` instead of specific types, and using exceptions for normal branching are the most prevalent anti-patterns.

## Why It's a Tech Debt Concern
Empty catch blocks silently discard exceptions, allowing corrupted state to propagate through the application undetected. Catching `Exception` or `Throwable` inadvertently catches `NullPointerException`, `OutOfMemoryError`, and other critical failures that should either crash or be handled explicitly. Using exceptions for control flow (e.g., catching `NumberFormatException` instead of validating input) adds unnecessary overhead and obscures the boundary between expected behavior and genuine errors.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`

---

## Pattern 1: Empty Catch Block

### Description
A `catch` block that contains no statements, or contains only a comment. The exception is caught and completely discarded, making it impossible to diagnose failures. This is especially dangerous with checked exceptions where the compiler forces a catch but the developer provides no implementation.

### Bad Code (Anti-pattern)
```java
public void loadConfiguration(String path) {
    try {
        Properties props = new Properties();
        props.load(new FileInputStream(path));
        this.config = props;
    } catch (IOException e) {
    }
}

public Connection getConnection() {
    try {
        return DriverManager.getConnection(url, user, password);
    } catch (SQLException e) {
        // TODO: handle later
    }
    return null;
}
```

### Good Code (Fix)
```java
public void loadConfiguration(String path) throws ConfigurationException {
    try {
        Properties props = new Properties();
        props.load(new FileInputStream(path));
        this.config = props;
    } catch (IOException e) {
        throw new ConfigurationException("Failed to load config from " + path, e);
    }
}

public Connection getConnection() throws DatabaseException {
    try {
        return DriverManager.getConnection(url, user, password);
    } catch (SQLException e) {
        logger.error("Database connection failed for {}", url, e);
        throw new DatabaseException("Could not connect to database", e);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `block`
- **Detection approach**: Find `catch_clause` nodes whose body `block` has zero child statements (excluding comments). Also flag blocks that contain only comments (all children are `line_comment` or `block_comment` nodes).
- **S-expression query sketch**:
```scheme
(catch_clause
  (catch_formal_parameter
    (catch_type
      (type_identifier) @exception_type)
    name: (identifier) @exception_var)
  body: (block) @catch_body)
```

### Pipeline Mapping
- **Pipeline name**: `exception_swallowing`
- **Pattern name**: `empty_catch_block`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Catching Exception or Throwable

### Description
Catching `Exception` or `Throwable` instead of specific exception types. `Exception` catches all checked and unchecked exceptions including `NullPointerException`, `ArrayIndexOutOfBoundsException`, and other bugs. `Throwable` additionally catches `Error` types like `OutOfMemoryError` and `StackOverflowError` that should almost never be caught.

### Bad Code (Anti-pattern)
```java
public UserProfile fetchProfile(long userId) {
    try {
        String json = httpClient.get("/api/users/" + userId);
        return objectMapper.readValue(json, UserProfile.class);
    } catch (Exception e) {
        logger.error("Failed to fetch profile", e);
        return null;
    }
}

public void processQueue() {
    while (running) {
        try {
            Message msg = queue.take();
            handler.handle(msg);
        } catch (Throwable t) {
            logger.error("Error processing message", t);
        }
    }
}
```

### Good Code (Fix)
```java
public UserProfile fetchProfile(long userId) throws ProfileException {
    try {
        String json = httpClient.get("/api/users/" + userId);
        return objectMapper.readValue(json, UserProfile.class);
    } catch (IOException e) {
        throw new ProfileException("HTTP request failed for user " + userId, e);
    } catch (JsonProcessingException e) {
        throw new ProfileException("Invalid profile JSON for user " + userId, e);
    }
}

public void processQueue() {
    while (running) {
        try {
            Message msg = queue.take();
            handler.handle(msg);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            logger.info("Queue processing interrupted");
            break;
        } catch (MessageHandlingException e) {
            logger.error("Failed to handle message: {}", e.getMessage(), e);
            deadLetterQueue.add(e.getMessage());
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `catch_clause`, `catch_formal_parameter`, `catch_type`, `type_identifier`
- **Detection approach**: Find `catch_clause` nodes where the `catch_type` contains a `type_identifier` with text equal to `Exception`, `Throwable`, or `RuntimeException`. These are the overly broad catch types. Optionally check for qualified names like `java.lang.Exception`.
- **S-expression query sketch**:
```scheme
(catch_clause
  (catch_formal_parameter
    (catch_type
      (type_identifier) @exception_type)))
```

### Pipeline Mapping
- **Pipeline name**: `exception_swallowing`
- **Pattern name**: `broad_exception_catch`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Exception Used for Control Flow

### Description
Using try/catch to handle expected conditions rather than checking preconditions. Common examples include parsing strings to numbers with `Integer.parseInt()` and catching `NumberFormatException`, or accessing collections and catching `IndexOutOfBoundsException` instead of checking bounds.

### Bad Code (Anti-pattern)
```java
public int parseAge(String input) {
    try {
        return Integer.parseInt(input);
    } catch (NumberFormatException e) {
        return -1;
    }
}

public String getElement(List<String> list, int index) {
    try {
        return list.get(index);
    } catch (IndexOutOfBoundsException e) {
        return null;
    }
}

public boolean isFeatureEnabled(Map<String, Object> config, String feature) {
    try {
        return (Boolean) config.get(feature);
    } catch (NullPointerException | ClassCastException e) {
        return false;
    }
}
```

### Good Code (Fix)
```java
public OptionalInt parseAge(String input) {
    if (input == null || input.isEmpty()) {
        return OptionalInt.empty();
    }
    try {
        int age = Integer.parseInt(input);
        return age >= 0 ? OptionalInt.of(age) : OptionalInt.empty();
    } catch (NumberFormatException e) {
        return OptionalInt.empty();
    }
}

public Optional<String> getElement(List<String> list, int index) {
    if (index >= 0 && index < list.size()) {
        return Optional.of(list.get(index));
    }
    return Optional.empty();
}

public boolean isFeatureEnabled(Map<String, Object> config, String feature) {
    Object value = config.get(feature);
    return value instanceof Boolean && (Boolean) value;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `catch_type`, `type_identifier`, `return_statement`
- **Detection approach**: Find `catch_clause` nodes where the caught type is `NumberFormatException`, `IndexOutOfBoundsException`, `NullPointerException`, or `ClassCastException` and the catch body contains a `return_statement` returning a default value (literal, null, empty optional). These indicate control flow usage. Check that the try body is short (1-3 statements) as a confidence booster.
- **S-expression query sketch**:
```scheme
(try_statement
  body: (block) @try_body
  (catch_clause
    (catch_formal_parameter
      (catch_type
        (type_identifier) @exception_type))
    body: (block
      (return_statement) @default_return)))
```

### Pipeline Mapping
- **Pipeline name**: `exception_swallowing`
- **Pattern name**: `exception_control_flow`
- **Severity**: info
- **Confidence**: medium
