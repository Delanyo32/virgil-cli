# Injection -- Java

## Overview
Injection vulnerabilities in Java arise when untrusted input is concatenated into SQL queries, passed to runtime command execution, or used to dynamically load classes via reflection. Java's verbose string handling (concatenation with `+`, `String.format`) and powerful runtime APIs (`Runtime.exec`, `Class.forName`) create multiple injection surfaces when user input is not properly validated.

## Why It's a Security Concern
SQL injection through `Statement.executeQuery()` can lead to full database compromise, including data exfiltration and privilege escalation. Command injection via `Runtime.exec()` grants attackers arbitrary OS command execution. Reflection injection through `Class.forName()` with user input can instantiate arbitrary classes, leading to remote code execution or denial of service. These are critical vulnerabilities in enterprise Java applications.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.sql (JDBC), Spring JDBC, Hibernate, java.lang.Runtime, java.lang.ProcessBuilder, java.lang.reflect

---

## Pattern 1: SQL Injection via String Concatenation in Statement.executeQuery

### Description
Building SQL queries by concatenating user-supplied strings with the `+` operator and executing them via `Statement.executeQuery()` or `Statement.execute()`. This bypasses the parameterized query protection offered by `PreparedStatement`.

### Bad Code (Anti-pattern)
```java
public User getUser(Connection conn, String userId) throws SQLException {
    Statement stmt = conn.createStatement();
    String query = "SELECT * FROM users WHERE id = '" + userId + "'";
    ResultSet rs = stmt.executeQuery(query);
    if (rs.next()) {
        return new User(rs.getString("id"), rs.getString("name"));
    }
    return null;
}
```

### Good Code (Fix)
```java
public User getUser(Connection conn, String userId) throws SQLException {
    PreparedStatement stmt = conn.prepareStatement("SELECT * FROM users WHERE id = ?");
    stmt.setString(1, userId);
    ResultSet rs = stmt.executeQuery();
    if (rs.next()) {
        return new User(rs.getString("id"), rs.getString("name"));
    }
    return null;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `binary_expression`, `string_literal`, `identifier`
- **Detection approach**: Find `method_invocation` nodes where the method name is `executeQuery`, `execute`, or `executeUpdate` and the argument is a `binary_expression` using `+` that includes a `string_literal` containing SQL keywords and an `identifier` or other non-literal expression. Also match when the argument is a variable previously assigned via string concatenation.
- **S-expression query sketch**:
```scheme
(method_invocation
  name: (identifier) @method
  arguments: (argument_list
    (binary_expression
      left: (string_literal) @sql_fragment
      right: (_) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_concat_statement`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via Runtime.exec

### Description
Passing user-controlled strings to `Runtime.getRuntime().exec()` either as a single command string (which is split by whitespace) or by interpolating user input into command arguments. `ProcessBuilder` with user-controlled command lists poses similar risks.

### Bad Code (Anti-pattern)
```java
public String runDiagnostic(String hostname) throws IOException {
    Process proc = Runtime.getRuntime().exec("ping -c 4 " + hostname);
    BufferedReader reader = new BufferedReader(new InputStreamReader(proc.getInputStream()));
    StringBuilder output = new StringBuilder();
    String line;
    while ((line = reader.readLine()) != null) {
        output.append(line).append("\n");
    }
    return output.toString();
}
```

### Good Code (Fix)
```java
public String runDiagnostic(String hostname) throws IOException {
    // Use array form to avoid shell interpretation
    ProcessBuilder pb = new ProcessBuilder("ping", "-c", "4", hostname);
    pb.redirectErrorStream(true);
    Process proc = pb.start();
    BufferedReader reader = new BufferedReader(new InputStreamReader(proc.getInputStream()));
    StringBuilder output = new StringBuilder();
    String line;
    while ((line = reader.readLine()) != null) {
        output.append(line).append("\n");
    }
    // Validate hostname against an allowlist or regex pattern
    return output.toString();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `binary_expression`, `string_literal`
- **Detection approach**: Find `method_invocation` chains where `Runtime.getRuntime().exec()` receives a `binary_expression` (string concatenation) as its argument. Also detect `ProcessBuilder` constructor calls where any element in the argument list is a non-literal expression. The single-string form of `exec()` is most dangerous as the JVM splits it on whitespace.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (method_invocation
    object: (method_invocation
      name: (identifier) @runtime_method)
    name: (identifier) @get_runtime)
  name: (identifier) @exec_method
  arguments: (argument_list
    (binary_expression) @cmd))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_runtime`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: Reflection Injection via Class.forName with User Input

### Description
Using `Class.forName()` with a user-supplied class name string to dynamically load and instantiate classes. An attacker can specify arbitrary classes, potentially triggering dangerous static initializers, instantiating exploitation gadgets, or accessing internal APIs.

### Bad Code (Anti-pattern)
```java
public Object createHandler(HttpServletRequest request) throws Exception {
    String className = request.getParameter("handler");
    Class<?> clazz = Class.forName(className);
    return clazz.getDeclaredConstructor().newInstance();
}
```

### Good Code (Fix)
```java
private static final Map<String, Class<?>> ALLOWED_HANDLERS = Map.of(
    "json", JsonHandler.class,
    "xml", XmlHandler.class,
    "csv", CsvHandler.class
);

public Object createHandler(HttpServletRequest request) throws Exception {
    String handlerName = request.getParameter("handler");
    Class<?> clazz = ALLOWED_HANDLERS.get(handlerName);
    if (clazz == null) {
        throw new IllegalArgumentException("Unknown handler: " + handlerName);
    }
    return clazz.getDeclaredConstructor().newInstance();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `identifier`, `argument_list`
- **Detection approach**: Find `method_invocation` nodes where the method is `forName` on `Class` (i.e., `Class.forName(...)`) and the argument is a variable or method call result (not a string literal). Trace the argument back to check if it originates from request parameters, user input, or external configuration.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @class_ref
  name: (identifier) @method
  arguments: (argument_list
    (identifier) @class_name_var))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `reflection_injection_forname`
- **Severity**: error
- **Confidence**: high
