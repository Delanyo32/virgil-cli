# Encapsulation Leaks -- Java

## Overview
Encapsulation leaks in Java occur when class fields are declared `public` without getters/setters, allowing any code to read and modify internal state directly, or when utility classes accumulate stateless `static` methods that should be instance methods on proper objects. Public fields bypass validation and invariant enforcement, while static utility sprawl produces procedural code disguised as OOP.

## Why It's a Tech Debt Concern
Mutable public fields make it impossible to add validation, logging, or change notification without modifying every access site across the codebase. They also prevent subclasses from overriding behavior and break the JavaBean convention that many frameworks (Spring, Hibernate, Jackson) rely on. Static utility sprawl creates rigid, untestable code — static methods cannot be overridden, mocked in unit tests, or substituted via dependency injection, leading to tightly coupled systems that resist change.

## Applicability
- **Relevance**: high (public fields and static utility classes are common in enterprise Java)
- **Languages covered**: `.java`
- **Frameworks/libraries**: Spring (dependency injection vs static), Hibernate/JPA (entity field access), Jackson (serialization conventions)

---

## Pattern 1: Mutable Public Fields

### Description
A class declares fields as `public` (non-final) instead of `private` with accessor methods. Any code with a reference to the object can read, modify, or set invalid values on these fields without the owning class being able to enforce constraints.

### Bad Code (Anti-pattern)
```java
public class UserAccount {
    public String username;
    public String email;
    public int age;
    public double balance;
    public List<String> roles;
    public Date lastLogin;
    public boolean active;

    public UserAccount(String username, String email) {
        this.username = username;
        this.email = email;
        this.roles = new ArrayList<>();
        this.active = true;
    }
}

// Any code can break invariants
UserAccount user = new UserAccount("alice", "alice@example.com");
user.age = -5;                   // no validation
user.balance = Double.MAX_VALUE; // no bounds check
user.email = "";                 // no format validation
user.roles.add("ADMIN");        // no authorization check
user.roles = null;               // breaks iteration elsewhere
user.active = false;             // no audit trail
```

### Good Code (Fix)
```java
public class UserAccount {
    private String username;
    private String email;
    private int age;
    private double balance;
    private List<String> roles;
    private Date lastLogin;
    private boolean active;

    public UserAccount(String username, String email) {
        setUsername(username);
        setEmail(email);
        this.roles = new ArrayList<>();
        this.active = true;
    }

    public String getUsername() { return username; }

    public void setUsername(String username) {
        if (username == null || username.isBlank()) {
            throw new IllegalArgumentException("Username cannot be blank");
        }
        this.username = username;
    }

    public String getEmail() { return email; }

    public void setEmail(String email) {
        if (email == null || !email.contains("@")) {
            throw new IllegalArgumentException("Invalid email");
        }
        this.email = email;
    }

    public int getAge() { return age; }

    public void setAge(int age) {
        if (age < 0 || age > 150) {
            throw new IllegalArgumentException("Invalid age: " + age);
        }
        this.age = age;
    }

    public double getBalance() { return balance; }

    public List<String> getRoles() {
        return Collections.unmodifiableList(roles);
    }

    public void addRole(String role) {
        Objects.requireNonNull(role);
        roles.add(role);
    }

    public boolean isActive() { return active; }

    public void deactivate() {
        this.active = false;
        this.lastLogin = null;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` inside `class_body` with `modifiers` containing `public` but not `final`
- **Detection approach**: Find `field_declaration` nodes inside `class_body` whose `modifiers` node contains the `public` modifier but does not contain `final`. Exclude `static` fields (those are Pattern 2 territory) and constants (`static final`). Flag classes with 2+ non-final public instance fields.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (class_body
      (field_declaration
        (modifiers
          (modifier) @mod)
        type: (_) @field_type
        declarator: (variable_declarator
          name: (identifier) @field_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `mutable_public_fields`
- **Pattern name**: `public_non_final_field`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Static Utility Sprawl

### Description
A class consists entirely of `static` methods with no instance state — a "utility class" that groups loosely related procedures. These methods cannot be overridden, mocked, or injected, creating tight coupling. Often the methods share implicit dependencies (database connections, configuration) passed as parameters, suggesting they should be instance methods on a proper service object.

### Bad Code (Anti-pattern)
```java
public class UserUtils {
    public static User findUser(Connection conn, int id) {
        // raw SQL query
    }

    public static void saveUser(Connection conn, User user) {
        // raw SQL insert
    }

    public static boolean validateEmail(String email) {
        return email != null && email.matches("^[\\w.]+@[\\w.]+$");
    }

    public static String hashPassword(String password) {
        return BCrypt.hashpw(password, BCrypt.gensalt());
    }

    public static void sendWelcomeEmail(SmtpClient client, User user) {
        client.send(user.getEmail(), "Welcome!", "Hello " + user.getUsername());
    }

    public static String generateToken(String secretKey, User user) {
        return Jwts.builder().setSubject(user.getUsername())
            .signWith(Keys.hmacShaKeyFor(secretKey.getBytes()))
            .compact();
    }

    public static boolean checkPermission(Connection conn, int userId, String resource) {
        // raw SQL query
    }

    public static List<User> searchUsers(Connection conn, String query) {
        // raw SQL query
    }
}

// Caller — tightly coupled, untestable
public class UserController {
    public void createUser(Request req) {
        UserUtils.validateEmail(req.getEmail());             // cannot mock
        String hash = UserUtils.hashPassword(req.getPassword()); // cannot mock
        UserUtils.saveUser(connection, user);                // cannot mock
        UserUtils.sendWelcomeEmail(smtpClient, user);       // cannot mock
    }
}
```

### Good Code (Fix)
```java
public class UserRepository {
    private final Connection conn;

    public UserRepository(Connection conn) {
        this.conn = conn;
    }

    public User findById(int id) { /* ... */ }
    public void save(User user) { /* ... */ }
    public List<User> search(String query) { /* ... */ }
}

public class AuthService {
    private final String secretKey;

    public AuthService(String secretKey) {
        this.secretKey = secretKey;
    }

    public String hashPassword(String password) {
        return BCrypt.hashpw(password, BCrypt.gensalt());
    }

    public String generateToken(User user) {
        return Jwts.builder().setSubject(user.getUsername())
            .signWith(Keys.hmacShaKeyFor(secretKey.getBytes()))
            .compact();
    }
}

public class EmailService {
    private final SmtpClient client;

    public EmailService(SmtpClient client) {
        this.client = client;
    }

    public void sendWelcome(User user) {
        client.send(user.getEmail(), "Welcome!", "Hello " + user.getUsername());
    }
}

// Caller — dependencies injected, testable
public class UserController {
    private final UserRepository repo;
    private final AuthService auth;
    private final EmailService email;

    public UserController(UserRepository repo, AuthService auth, EmailService email) {
        this.repo = repo;
        this.auth = auth;
        this.email = email;
    }

    public void createUser(Request req) {
        String hash = auth.hashPassword(req.getPassword());
        repo.save(user);
        email.sendWelcome(user);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration` containing only `static` `method_declaration` nodes, no instance fields
- **Detection approach**: Find `class_declaration` nodes where all `method_declaration` children in the `class_body` have `static` in their `modifiers`, and the class has no non-static `field_declaration` nodes. Flag classes with 4+ static methods and zero instance state. Exclude classes with a private constructor (intentional utility class pattern like `Math`).
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (class_body
      (method_declaration
        (modifiers
          (modifier) @mod)
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `static_utility_sprawl`
- **Pattern name**: `static_only_class`
- **Severity**: info
- **Confidence**: medium
