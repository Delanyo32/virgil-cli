# Dead Code -- Java

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. It also increases compilation time and complicates refactoring. Java's compiler catches some unreachable code as errors, but unused private methods and commented-out code slip through.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Private Method

### Description
A private method defined but never called from anywhere in the class. Java's compiler does not warn on unused private methods.

### Bad Code (Anti-pattern)
```java
public class UserService {

    private String sanitizeLegacyInput(String input) {
        return input.replaceAll("[^a-zA-Z0-9]", "")
                    .toLowerCase()
                    .trim();
    }

    public User createUser(String name, String email) {
        return new User(name, email);
    }
}
```

### Good Code (Fix)
```java
public class UserService {

    public User createUser(String name, String email) {
        return new User(name, email);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`
- **Detection approach**: Collect all private method definitions (methods whose `modifiers` node contains the `private` keyword). Cross-reference with all `method_invocation` nodes within the same class. Private methods with zero references are candidates. Exclude methods annotated with `@Override`, `@Bean`, `@EventListener`, `@Scheduled`, or reflection-based frameworks. Also exclude methods matching serialization conventions (`readObject`, `writeObject`, `readResolve`).
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    (modifiers "private") @mod
    name: (identifier) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Throw

### Description
Code statements that appear after an unconditional return, throw, break, or continue — they can never execute. Java's compiler catches some of these as errors, but certain patterns survive, especially after `System.exit()`.

### Bad Code (Anti-pattern)
```java
public String resolveEndpoint(String env) {
    switch (env) {
        case "prod":
            return "https://api.example.com";
        case "staging":
            return "https://staging.example.com";
        default:
            throw new IllegalArgumentException("Unknown env: " + env);
            // return "http://localhost:8080"; // unreachable
    }
}

public void shutdown() {
    saveState();
    System.exit(0);
    logger.info("Shutdown complete"); // unreachable — System.exit terminates JVM
}
```

### Good Code (Fix)
```java
public String resolveEndpoint(String env) {
    switch (env) {
        case "prod":
            return "https://api.example.com";
        case "staging":
            return "https://staging.example.com";
        default:
            throw new IllegalArgumentException("Unknown env: " + env);
    }
}

public void shutdown() {
    saveState();
    System.exit(0);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `throw_statement`, `break_statement`, `continue_statement`, `expression_statement` (for `System.exit()`)
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `block`. Java's compiler rejects obviously unreachable code, but patterns involving `System.exit()` or complex control flow may slip through. Also check `switch` cases where a `return`/`throw` is followed by a `break`.
- **S-expression query sketch**:
  ```scheme
  (block
    (return_statement) @exit
    .
    (_) @unreachable)
  (block
    (throw_statement) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```java
public class OrderProcessor {

    public Receipt process(Order order) {
        // private boolean validateInventory(Order order) {
        //     for (LineItem item : order.getItems()) {
        //         int available = inventoryService.getStock(item.getSku());
        //         if (available < item.getQuantity()) {
        //             return false;
        //         }
        //     }
        //     return true;
        // }
        //
        // if (!validateInventory(order)) {
        //     throw new OutOfStockException(order.getId());
        // }

        return chargeAndFulfill(order);
    }
}
```

### Good Code (Fix)
```java
public class OrderProcessor {

    public Receipt process(Order order) {
        return chargeAndFulfill(order);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `line_comment`, `block_comment`
- **Detection approach**: Find comment nodes whose content matches Java code patterns (contains `public `, `private `, `class `, `return `, `new `, `if (`, `for (`, `.get`, semicolons at end of lines). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from Javadoc (`/** */`), license headers, and TODO comments.
- **S-expression query sketch**:
  ```scheme
  (line_comment) @comment
  (block_comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
