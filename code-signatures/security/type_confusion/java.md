# Type Confusion -- Java

## Overview
Java's type system is statically checked at compile time, but runtime casts using `(Type) object` bypass static checking and defer type validation to runtime. When an object is cast to an incompatible type without a preceding `instanceof` check, a `ClassCastException` is thrown. In server applications, unhandled `ClassCastException` can cause request failures, information leakage through stack traces, and denial of service.

## Why It's a Security Concern
Unsafe casts are common when working with raw types (pre-generics code), `Object` parameters, deserialized data, or reflection results. An attacker who controls the type of an object -- through crafted serialized payloads, plugin systems, or dependency injection -- can trigger `ClassCastException` at critical points, causing denial of service or bypassing security logic that assumes a cast always succeeds. Stack traces from `ClassCastException` can also reveal internal class names, package structures, and framework versions.

## Applicability
- **Relevance**: medium
- **Languages covered**: .java
- **Frameworks/libraries**: javax.servlet, Spring (dependency injection, deserialization), Jackson, Java serialization (ObjectInputStream), reflection API

---

## Pattern 1: Unsafe Cast Without instanceof Check

### Description
Casting an `Object` reference to a specific type using `(Type) object` without first verifying the type with `instanceof`. This is particularly dangerous when the object comes from external sources: deserialized data, HTTP request attributes, session stores, or generic collections that lost type information due to type erasure.

### Bad Code (Anti-pattern)
```java
public class RequestHandler {
    public void handleRequest(HttpServletRequest request) {
        // Throws ClassCastException if attribute is wrong type or null
        UserSession session = (UserSession) request.getAttribute("session");
        String role = session.getRole();

        // Unsafe cast from deserialized object
        Object payload = deserialize(request.getInputStream());
        Map<String, Object> data = (Map<String, Object>) payload;
        String command = (String) data.get("action");

        executeCommand(command, session);
    }

    public void processMessage(Object message) {
        // Direct cast without type check -- crashes on unexpected types
        CommandMessage cmd = (CommandMessage) message;
        cmd.execute();
    }
}
```

### Good Code (Fix)
```java
public class RequestHandler {
    public void handleRequest(HttpServletRequest request) {
        Object sessionAttr = request.getAttribute("session");
        if (!(sessionAttr instanceof UserSession)) {
            response.sendError(HttpServletResponse.SC_UNAUTHORIZED);
            return;
        }
        UserSession session = (UserSession) sessionAttr;
        String role = session.getRole();

        Object payload = deserialize(request.getInputStream());
        if (!(payload instanceof Map<?, ?> rawMap)) {
            response.sendError(HttpServletResponse.SC_BAD_REQUEST);
            return;
        }
        // Validate individual entries
        Object actionObj = rawMap.get("action");
        if (!(actionObj instanceof String command)) {
            response.sendError(HttpServletResponse.SC_BAD_REQUEST);
            return;
        }

        executeCommand(command, session);
    }

    public void processMessage(Object message) {
        if (message instanceof CommandMessage cmd) {
            cmd.execute();
        } else {
            logger.warn("Unexpected message type: {}", message.getClass().getName());
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `cast_expression`, `parenthesized_type`, `identifier`, `method_invocation`
- **Detection approach**: Find `cast_expression` nodes (the `(Type) expr` syntax) where the target type is not a primitive type. Check whether the cast is preceded by an `instanceof` check for the same variable and type within the enclosing block or `if` condition. Flag casts that lack a corresponding `instanceof` guard, especially when the source expression is a method call returning `Object` (e.g., `getAttribute()`, `get()`, `readObject()`).
- **S-expression query sketch**:
```scheme
(cast_expression
  type: (_) @cast_type
  value: (_) @cast_value)
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `unsafe_cast`
- **Severity**: warning
- **Confidence**: medium
