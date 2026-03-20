# Magic Values -- Java

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```java
public class RequestHandler {
    public Response processRequest(byte[] data) {
        if (data.length > 1024) {
            return new Response(413);
        }
        for (int i = 0; i < 3; i++) {
            Thread.sleep(86400 * 1000L);
        }
        if (response.getStatusCode() == 200) {
            cache.put(key, data, 3600);
        } else if (response.getStatusCode() == 404) {
            return Response.notFound();
        }
        return Response.ok();
    }
}
```

### Good Code (Fix)
```java
public class RequestHandler {
    private static final int MAX_PAYLOAD_SIZE = 1024;
    private static final int MAX_RETRIES = 3;
    private static final long SECONDS_PER_DAY = 86400L;
    private static final long MS_PER_SECOND = 1000L;
    private static final int HTTP_OK = 200;
    private static final int HTTP_NOT_FOUND = 404;
    private static final int CACHE_TTL_SECONDS = 3600;

    public Response processRequest(byte[] data) {
        if (data.length > MAX_PAYLOAD_SIZE) {
            return new Response(HTTP_PAYLOAD_TOO_LARGE);
        }
        for (int i = 0; i < MAX_RETRIES; i++) {
            Thread.sleep(SECONDS_PER_DAY * MS_PER_SECOND);
        }
        if (response.getStatusCode() == HTTP_OK) {
            cache.put(key, data, CACHE_TTL_SECONDS);
        } else if (response.getStatusCode() == HTTP_NOT_FOUND) {
            return Response.notFound();
        }
        return Response.ok();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `decimal_integer_literal`, `decimal_floating_point_literal`, `hex_integer_literal`, `octal_integer_literal` (excludes 0, 1, -1)
- **Detection approach**: Find numeric literal nodes in expressions. Exclude literals inside `field_declaration` ancestors that have `static final` modifiers (constant definitions), `enum_constant` ancestors, and `annotation` ancestors. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
[(decimal_integer_literal) @number
 (decimal_floating_point_literal) @number
 (hex_integer_literal) @number]
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```java
public class UserService {
    public void handleUser(User user) {
        if (user.getRole().equals("admin")) {
            grantAccess("dashboard");
        }
        if (user.getStatus().equals("active") || user.getStatus().equals("pending")) {
            notify(user);
        }
        String dbUrl = config.get("database_url");
    }
}
```

### Good Code (Fix)
```java
public class UserService {
    private static final String ROLE_ADMIN = "admin";
    private static final String STATUS_ACTIVE = "active";
    private static final String STATUS_PENDING = "pending";
    private static final String CONFIG_DATABASE_URL = "database_url";

    public void handleUser(User user) {
        if (user.getRole().equals(ROLE_ADMIN)) {
            grantAccess("dashboard");
        }
        if (user.getStatus().equals(STATUS_ACTIVE) || user.getStatus().equals(STATUS_PENDING)) {
            notify(user);
        }
        String dbUrl = config.get(CONFIG_DATABASE_URL);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string_literal` in `method_invocation` (`.equals()`, `.equalsIgnoreCase()` calls) or `argument_list` (map/config lookup arguments)
- **Detection approach**: Find `string_literal` nodes used as arguments to `.equals()` and `.equalsIgnoreCase()` method invocations, or as arguments to `.get()`, `.put()`, `.containsKey()` map methods. Exclude empty strings, logging messages, and SQL strings. Flag non-empty string literals in equality comparisons.
- **S-expression query sketch**:
```scheme
(method_invocation
  name: (identifier) @method_name
  arguments: (argument_list
    (string_literal) @string_lit)) @invocation
```

### Pipeline Mapping
- **Pipeline name**: `magic_strings`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
