# Resource Lifecycle -- Java

## Overview
Resources that are acquired but never properly released cause memory leaks, connection pool exhaustion, and file descriptor leaks. In Java, the most common manifestations are `AutoCloseable` resources not wrapped in try-with-resources blocks and JDBC connections or statements leaked in error paths.

## Why It's a Tech Debt Concern
Java's `AutoCloseable` interface and try-with-resources (introduced in Java 7) provide a reliable mechanism for resource cleanup, but legacy code and developer habit often bypass them. Resources like `InputStream`, `Connection`, `Statement`, and `ResultSet` that are not closed in all code paths (including exception paths) accumulate until the JVM runs out of file descriptors or the database connection pool is exhausted. JDBC leaks are especially dangerous because database connections are expensive to create and limited in number -- a single leaked connection per request can bring down a production system within minutes under load.

## Applicability
- **Relevance**: high (JDBC, file I/O, network connections)
- **Languages covered**: `.java`
- **Frameworks/libraries**: JDBC, java.io, java.nio, Apache HttpClient, OkHttp

---

## Pattern 1: AutoCloseable Not in try-with-resources

### Description
Creating an `AutoCloseable` resource (such as `InputStream`, `OutputStream`, `Reader`, `Writer`, `Connection`, `Socket`) with a manual `try/finally` or without any cleanup guarantee. If an exception occurs between resource acquisition and the explicit `.close()` call, the resource leaks.

### Bad Code (Anti-pattern)
```java
// Manual close -- exception before close leaks the stream
public String readFile(String path) throws IOException {
    FileInputStream fis = new FileInputStream(path);
    BufferedReader reader = new BufferedReader(new InputStreamReader(fis));
    String content = reader.readLine();
    reader.close(); // Never reached if readLine() throws
    return content;
}

// Close in finally but inner resource leaked
public void copyFile(String src, String dst) throws IOException {
    InputStream in = new FileInputStream(src);
    OutputStream out = null;
    try {
        out = new FileOutputStream(dst);
        byte[] buf = new byte[8192];
        int len;
        while ((len = in.read(buf)) > 0) {
            out.write(buf, 0, len);
        }
    } finally {
        if (out != null) out.close();
        in.close(); // If out.close() throws, in is leaked
    }
}

// Socket never closed on error path
public String fetchUrl(String host, int port) throws IOException {
    Socket socket = new Socket(host, port);
    InputStream stream = socket.getInputStream();
    byte[] data = stream.readAllBytes();
    socket.close();
    return new String(data);
}
```

### Good Code (Fix)
```java
// try-with-resources ensures close on all paths
public String readFile(String path) throws IOException {
    try (var fis = new FileInputStream(path);
         var reader = new BufferedReader(new InputStreamReader(fis))) {
        return reader.readLine();
    }
}

// Multiple resources in try-with-resources
public void copyFile(String src, String dst) throws IOException {
    try (var in = new FileInputStream(src);
         var out = new FileOutputStream(dst)) {
        byte[] buf = new byte[8192];
        int len;
        while ((len = in.read(buf)) > 0) {
            out.write(buf, 0, len);
        }
    }
}

// Socket properly managed
public String fetchUrl(String host, int port) throws IOException {
    try (var socket = new Socket(host, port);
         var stream = socket.getInputStream()) {
        return new String(stream.readAllBytes());
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_variable_declaration`, `object_creation_expression`, `try_with_resources_statement`
- **Detection approach**: Find `object_creation_expression` nodes creating instances of known `AutoCloseable` types (`FileInputStream`, `BufferedReader`, `Socket`, `Connection`, etc.) that are assigned in a `local_variable_declaration`. Check if the declaration is inside a `try_with_resources_statement`'s resource specification. Flag declarations outside try-with-resources where the type is a known closeable.
- **S-expression query sketch**:
  ```scheme
  ;; Resource creation outside try-with-resources
  (local_variable_declaration
    declarator: (variable_declarator
      name: (identifier) @var_name
      value: (object_creation_expression
        type: (type_identifier) @type_name)))

  ;; Safe: resource in try-with-resources
  (try_with_resources_statement
    resources: (resource_specification
      (resource
        name: (identifier) @var_name
        value: (object_creation_expression
          type: (type_identifier) @type_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `resource_leaks`
- **Pattern name**: `autocloseable_not_in_try_with_resources`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: JDBC Connection/Statement Leak in Error Paths

### Description
Acquiring a JDBC `Connection`, `Statement`, or `PreparedStatement` and executing queries without ensuring all three resources are closed in error paths. A common pattern is closing the `ResultSet` but forgetting the `Statement`, or closing in the happy path but not when an `SQLException` is thrown. Connection pool exhaustion from this pattern typically surfaces only under production load.

### Bad Code (Anti-pattern)
```java
// Connection and statement leaked if executeQuery() throws
public List<User> getUsers(DataSource ds) throws SQLException {
    Connection conn = ds.getConnection();
    Statement stmt = conn.createStatement();
    ResultSet rs = stmt.executeQuery("SELECT * FROM users");
    List<User> users = new ArrayList<>();
    while (rs.next()) {
        users.add(new User(rs.getInt("id"), rs.getString("name")));
    }
    rs.close();
    stmt.close();
    conn.close();
    return users;
}

// PreparedStatement leaked on exception
public void insertUser(DataSource ds, String name) throws SQLException {
    Connection conn = ds.getConnection();
    try {
        PreparedStatement ps = conn.prepareStatement("INSERT INTO users (name) VALUES (?)");
        ps.setString(1, name);
        ps.executeUpdate();
        ps.close();
    } finally {
        conn.close();
        // ps is leaked if executeUpdate() throws
    }
}

// ResultSet leaked -- only connection closed in finally
public int countRecords(DataSource ds, String table) throws SQLException {
    Connection conn = ds.getConnection();
    try {
        Statement stmt = conn.createStatement();
        ResultSet rs = stmt.executeQuery("SELECT COUNT(*) FROM " + table);
        rs.next();
        return rs.getInt(1);
    } finally {
        conn.close(); // stmt and rs are leaked
    }
}
```

### Good Code (Fix)
```java
// All resources in try-with-resources
public List<User> getUsers(DataSource ds) throws SQLException {
    try (Connection conn = ds.getConnection();
         Statement stmt = conn.createStatement();
         ResultSet rs = stmt.executeQuery("SELECT * FROM users")) {
        List<User> users = new ArrayList<>();
        while (rs.next()) {
            users.add(new User(rs.getInt("id"), rs.getString("name")));
        }
        return users;
    }
}

// PreparedStatement in try-with-resources
public void insertUser(DataSource ds, String name) throws SQLException {
    try (Connection conn = ds.getConnection();
         PreparedStatement ps = conn.prepareStatement("INSERT INTO users (name) VALUES (?)")) {
        ps.setString(1, name);
        ps.executeUpdate();
    }
}

// All three resources managed
public int countRecords(DataSource ds, String table) throws SQLException {
    try (Connection conn = ds.getConnection();
         Statement stmt = conn.createStatement();
         ResultSet rs = stmt.executeQuery("SELECT COUNT(*) FROM " + table)) {
        rs.next();
        return rs.getInt(1);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_variable_declaration`, `method_invocation`, `try_with_resources_statement`
- **Detection approach**: Find `method_invocation` nodes calling `getConnection()`, `createStatement()`, `prepareStatement()`, or `executeQuery()` whose return value is assigned in a `local_variable_declaration`. Check if the declaration is inside a `try_with_resources_statement` resource specification. Flag JDBC resource acquisitions that are in plain `try` or outside any `try` block entirely.
- **S-expression query sketch**:
  ```scheme
  ;; JDBC resource acquisition
  (local_variable_declaration
    declarator: (variable_declarator
      name: (identifier) @var_name
      value: (method_invocation
        name: (identifier) @method_name
        object: (identifier) @source_obj)))
  ```

### Pipeline Mapping
- **Pipeline name**: `resource_leaks`
- **Pattern name**: `jdbc_leak_in_error_path`
- **Severity**: warning
- **Confidence**: high
