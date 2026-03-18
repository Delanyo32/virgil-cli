# Resource Exhaustion -- Java

## Overview
Resource exhaustion vulnerabilities in Java arise from Regular Expression Denial of Service (ReDoS) via `Pattern.compile()` with backtracking-prone patterns, and from unbounded creation of threads, connections, or other system resources driven by user-controlled input. Java's `java.util.regex` engine uses an NFA backtracking implementation susceptible to catastrophic backtracking, and the JVM's thread model makes unbounded thread creation a reliable denial-of-service vector.

## Why It's a Security Concern
ReDoS in Java can lock up servlet threads for minutes or hours per request, exhausting the thread pool and making the application completely unresponsive. Unbounded thread or resource creation allows attackers to exhaust JVM heap memory, OS thread limits, or file descriptor limits by sending requests that trigger uncontrolled resource allocation. Both attacks can be mounted with minimal effort and cause total service outage.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.util.regex, java.lang.Thread, java.util.concurrent, Spring, Jakarta Servlet

---

## Pattern 1: ReDoS -- Pattern.compile with Catastrophic Backtracking

### Description
Compiling and applying a regular expression containing nested quantifiers, overlapping alternations, or other backtracking-prone constructs via `Pattern.compile()` or `String.matches()` against user-supplied input. Java's regex engine does not have a built-in timeout (prior to JDK 9's limited interrupt support), making it especially vulnerable.

### Bad Code (Anti-pattern)
```java
import java.util.regex.Pattern;
import java.util.regex.Matcher;
import javax.servlet.http.HttpServletRequest;

public class InputValidator {
    // Nested quantifiers cause catastrophic backtracking
    private static final Pattern EMAIL_PATTERN =
        Pattern.compile("^([a-zA-Z0-9_\\.\\-]+)+@([a-zA-Z0-9\\-]+\\.)+[a-zA-Z]{2,}$");

    public boolean validateEmail(HttpServletRequest request) {
        String email = request.getParameter("email");
        Matcher matcher = EMAIL_PATTERN.matcher(email);
        return matcher.matches();
    }

    public boolean validateTag(String userInput) {
        // Overlapping groups with quantifiers
        return userInput.matches("(<[^>]*>)*.*(<\\/[^>]*>)*");
    }
}
```

### Good Code (Fix)
```java
import java.util.regex.Pattern;
import java.util.regex.Matcher;
import javax.servlet.http.HttpServletRequest;

public class InputValidator {
    // Linear-time pattern without nested quantifiers
    private static final Pattern EMAIL_PATTERN =
        Pattern.compile("^[a-zA-Z0-9._%+\\-]+@[a-zA-Z0-9.\\-]+\\.[a-zA-Z]{2,}$");

    private static final int MAX_INPUT_LENGTH = 254;

    public boolean validateEmail(HttpServletRequest request) {
        String email = request.getParameter("email");
        if (email == null || email.length() > MAX_INPUT_LENGTH) {
            return false;
        }
        Matcher matcher = EMAIL_PATTERN.matcher(email);
        return matcher.matches();
    }

    public boolean validateTag(String userInput) {
        if (userInput.length() > 10_000) {
            return false;
        }
        // Use a proper XML/HTML parser instead of regex for tag matching
        return isWellFormedXml(userInput);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `object_creation_expression`, `string_literal`, `identifier`
- **Detection approach**: Find `method_invocation` nodes calling `Pattern.compile()` or `String.matches()`. Extract the regex pattern from the first string literal argument. Analyze the pattern for nested quantifiers -- groups containing `+` or `*` that are themselves quantified with `+` or `*`. Also detect `Pattern.compile()` calls where the pattern is loaded from a variable (runtime-constructed patterns are inherently riskier).
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @class
  name: (identifier) @method
  arguments: (argument_list
    (string_literal) @pattern))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `regex_catastrophic_backtracking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Unbounded Thread/Resource Creation from User Input

### Description
Creating new `Thread` objects, `ExecutorService` thread pools, database connections, or other heavyweight resources in a loop or handler where the count or frequency is controlled by user input. Without upper bounds, attackers can exhaust JVM memory (each thread uses ~512KB-1MB of stack) or OS-level resource limits.

### Bad Code (Anti-pattern)
```java
import javax.servlet.http.HttpServletRequest;
import javax.servlet.http.HttpServletResponse;

public class BatchProcessor {
    public void handleBatch(HttpServletRequest request, HttpServletResponse response) {
        int count = Integer.parseInt(request.getParameter("count"));
        // User controls thread count -- could be millions
        for (int i = 0; i < count; i++) {
            new Thread(() -> {
                processItem(i);
            }).start();
        }
        response.setStatus(202);
    }

    public void processConnections(int numConnections) {
        // Unbounded connection creation
        for (int i = 0; i < numConnections; i++) {
            Connection conn = DriverManager.getConnection(DB_URL);
            processWithConnection(conn);
        }
    }
}
```

### Good Code (Fix)
```java
import javax.servlet.http.HttpServletRequest;
import javax.servlet.http.HttpServletResponse;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Semaphore;

public class BatchProcessor {
    private static final int MAX_BATCH_SIZE = 1000;
    private static final ExecutorService executor = Executors.newFixedThreadPool(20);
    private static final Semaphore semaphore = new Semaphore(100);

    public void handleBatch(HttpServletRequest request, HttpServletResponse response) {
        int count;
        try {
            count = Integer.parseInt(request.getParameter("count"));
        } catch (NumberFormatException e) {
            response.setStatus(400);
            return;
        }
        if (count > MAX_BATCH_SIZE) {
            response.setStatus(400);
            return;
        }
        for (int i = 0; i < count; i++) {
            final int idx = i;
            executor.submit(() -> {
                processItem(idx);
            });
        }
        response.setStatus(202);
    }

    public void processConnections(int numConnections) throws Exception {
        int bounded = Math.min(numConnections, 50);
        // Use connection pool instead of raw connections
        for (int i = 0; i < bounded; i++) {
            semaphore.acquire();
            try {
                Connection conn = dataSource.getConnection(); // From pool
                processWithConnection(conn);
            } finally {
                semaphore.release();
            }
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `method_invocation`, `for_statement`, `identifier`
- **Detection approach**: Find `object_creation_expression` nodes creating `new Thread(...)` inside `for_statement` loops where the loop bound is a variable (not a constant). Also detect calls to `Executors.newCachedThreadPool()` (unbounded by design) or `DriverManager.getConnection()` inside loops with user-controlled bounds. Flag when no preceding validation caps the loop count.
- **S-expression query sketch**:
```scheme
(for_statement
  condition: (binary_expression
    right: (identifier) @bound)
  body: (block
    (expression_statement
      (object_creation_expression
        type: (type_identifier) @type))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_thread_creation`
- **Severity**: warning
- **Confidence**: medium
