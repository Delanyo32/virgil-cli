# Type Safety Gaps -- Java

## Overview
Java's generics system provides compile-time type safety, but raw generic types and missing `final` modifiers create gaps where the compiler cannot enforce correctness. Raw types bypass generic type checking entirely, and mutable fields/parameters invite unintended reassignment that leads to subtle bugs.

## Why It's a Tech Debt Concern
Using raw generic types like `List` instead of `List<String>` disables all generic type checking at the use site, allowing any object to be inserted and requiring unsafe casts on retrieval. The compiler issues unchecked warnings, but these are frequently suppressed. Missing `final` on fields and parameters that should be immutable allows accidental reassignment, makes reasoning about state harder in concurrent code, and prevents the compiler from flagging unintended mutations.

## Applicability
- **Relevance**: high (raw types are common in legacy code; missing `final` is pervasive)
- **Languages covered**: `.java`
- **Frameworks/libraries**: All Java codebases; common in Spring, Hibernate, and legacy enterprise code

---

## Pattern 1: Raw Generic Types

### Description
Using generic classes without type parameters -- `List`, `Map`, `Set`, `Optional`, `Class`, `Iterator` -- instead of parameterized forms like `List<String>`. Raw types erase all generic type information, allowing type-unsafe operations that only fail at runtime with `ClassCastException`.

### Bad Code (Anti-pattern)
```java
public class DataProcessor {
    private List items = new ArrayList();
    private Map lookup = new HashMap();

    public void addItem(Object item) {
        items.add(item);
    }

    public List processAll() {
        List results = new ArrayList();
        for (Object item : items) {
            String value = (String) item;  // ClassCastException at runtime
            results.add(value.toUpperCase());
        }
        return results;
    }

    public void buildLookup(List entries) {
        for (Object entry : entries) {
            Map.Entry e = (Map.Entry) entry;
            lookup.put(e.getKey(), e.getValue());
        }
    }
}
```

### Good Code (Fix)
```java
public class DataProcessor {
    private final List<String> items = new ArrayList<>();
    private final Map<String, Integer> lookup = new HashMap<>();

    public void addItem(String item) {
        items.add(item);
    }

    public List<String> processAll() {
        List<String> results = new ArrayList<>();
        for (String item : items) {
            results.add(item.toUpperCase());
        }
        return results;
    }

    public void buildLookup(List<Map.Entry<String, Integer>> entries) {
        for (Map.Entry<String, Integer> entry : entries) {
            lookup.put(entry.getKey(), entry.getValue());
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `type_identifier`, `generic_type`, `field_declaration`, `local_variable_declaration`, `formal_parameter`
- **Detection approach**: Find `type_identifier` nodes used in variable declarations, field declarations, method parameters, and return types where the identifier is a known generic class (`List`, `Map`, `Set`, `Collection`, `Iterator`, `Optional`, `Class`, `Comparable`, `Future`, `Supplier`, `Function`, `Consumer`) but is not wrapped in a `generic_type` node (which would include type arguments). Flag these as raw type usage.
- **S-expression query sketch**:
```scheme
(field_declaration
  type: (type_identifier) @raw_type)

(local_variable_declaration
  type: (type_identifier) @raw_type)

(formal_parameter
  type: (type_identifier) @raw_type)
```

### Pipeline Mapping
- **Pipeline name**: `raw_types`
- **Pattern name**: `raw_generic_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Missing `final` on Fields and Parameters

### Description
Fields and method parameters that should not be reassigned after initialization are declared without the `final` modifier. This allows accidental reassignment, complicates reasoning about object state (especially in concurrent code), and prevents the compiler from catching unintended mutations.

### Bad Code (Anti-pattern)
```java
public class UserService {
    private UserRepository repository;
    private EmailService emailService;
    private int maxRetries;

    public UserService(UserRepository repository, EmailService emailService) {
        this.repository = repository;
        this.emailService = emailService;
        this.maxRetries = 3;
    }

    public User findUser(String id, boolean includeProfile) {
        User user = repository.findById(id);
        if (includeProfile) {
            user = enrichWithProfile(user);  // Reassigns parameter-like local
        }
        return user;
    }

    public void processUsers(List<User> users) {
        for (int i = 0; i < users.size(); i++) {
            User user = users.get(i);
            sendNotification(user);
        }
    }
}
```

### Good Code (Fix)
```java
public class UserService {
    private final UserRepository repository;
    private final EmailService emailService;
    private final int maxRetries;

    public UserService(final UserRepository repository, final EmailService emailService) {
        this.repository = repository;
        this.emailService = emailService;
        this.maxRetries = 3;
    }

    public User findUser(final String id, final boolean includeProfile) {
        final User user = repository.findById(id);
        if (includeProfile) {
            return enrichWithProfile(user);
        }
        return user;
    }

    public void processUsers(final List<User> users) {
        for (int i = 0; i < users.size(); i++) {
            final User user = users.get(i);
            sendNotification(user);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration`, `formal_parameter`, `modifiers`
- **Detection approach**: Find `field_declaration` nodes that are assigned in a constructor and never reassigned elsewhere. Check whether their `modifiers` child contains `final`. For method parameters, find `formal_parameter` nodes whose `modifiers` (if any) do not contain `final`. Flag fields that are only assigned once (in declaration or constructor) and lack `final`, and all non-final parameters.
- **S-expression query sketch**:
```scheme
(field_declaration
  (modifiers) @mods
  type: (_) @type
  declarator: (variable_declarator
    name: (identifier) @field_name))

(formal_parameter
  (modifiers)? @mods
  type: (_) @type
  name: (identifier) @param_name)
```

### Pipeline Mapping
- **Pipeline name**: `missing_final`
- **Pattern name**: `missing_final_modifier`
- **Severity**: info
- **Confidence**: medium
