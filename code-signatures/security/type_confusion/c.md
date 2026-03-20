# Type Confusion -- C

## Overview
C's type system allows unrestricted casting between pointer types, with `void*` serving as a universal pointer type that can be cast to any other pointer type without compiler checks. When a `void*` pointer is cast to the wrong type, the program reinterprets memory as a different structure, leading to undefined behavior, data corruption, and exploitable vulnerabilities.

## Why It's a Security Concern
Incorrect pointer casts are a primary source of type confusion vulnerabilities in C codebases, particularly in operating system kernels, network protocol parsers, and interpreters. When a `void*` is cast to the wrong struct type, subsequent field accesses read from incorrect memory offsets -- potentially exposing sensitive data, corrupting heap metadata, or overwriting function pointers. Attackers who can influence which type a `void*` is cast to (through protocol confusion, object type fields, or callback registration) can achieve arbitrary code execution. This class of vulnerability has been exploited in browser engines, kernel subsystems, and protocol implementations.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: Linux kernel, libc, OpenSSL, any C library using void* for generic data structures or callback user data

---

## Pattern 1: Casting Between Incompatible Pointer Types

### Description
Casting a `void*` pointer to a concrete struct or data type without validating that the pointer actually refers to the expected type. This is common in generic data structures (linked lists, hash tables), callback systems (where `void* user_data` is passed), and protocol parsers (where a type tag determines interpretation). If the type tag is attacker-controlled or the wrong cast is selected through a logic error, the program accesses memory with incorrect layout assumptions.

### Bad Code (Anti-pattern)
```c
struct animal {
    int type;
    char name[32];
};

struct vehicle {
    int type;
    int wheels;
    double weight;
    void (*start)(struct vehicle *);
};

void process_entity(void *entity, int type_tag) {
    // Trusts the type_tag without validation
    if (type_tag == TYPE_VEHICLE) {
        // If entity is actually a struct animal, this reads garbage for wheels/weight
        // and calls a garbage function pointer via start()
        struct vehicle *v = (struct vehicle *)entity;
        v->start(v);  // potential arbitrary code execution
    }
}

// Generic list with no type safety
struct list_node {
    void *data;
    struct list_node *next;
};

void process_users(struct list_node *head) {
    while (head) {
        // Assumes all nodes contain struct user -- no validation
        struct user *u = (struct user *)head->data;
        grant_access(u->username, u->role);
        head = head->next;
    }
}

// Callback with void* user_data
void on_event(void *user_data) {
    // Wrong cast if caller registered with a different type
    struct config *cfg = (struct config *)user_data;
    apply_config(cfg->settings);  // reads wrong memory
}
```

### Good Code (Fix)
```c
// Use a tagged union or type-discriminated struct
struct entity {
    enum entity_type type;
    union {
        struct animal animal;
        struct vehicle vehicle;
    } data;
};

void process_entity(struct entity *entity) {
    switch (entity->type) {
        case TYPE_VEHICLE:
            // Type is part of the struct -- cannot be forged separately
            entity->data.vehicle.start(&entity->data.vehicle);
            break;
        case TYPE_ANIMAL:
            printf("Animal: %s\n", entity->data.animal.name);
            break;
        default:
            fprintf(stderr, "Unknown entity type: %d\n", entity->type);
            break;
    }
}

// Type-safe list with embedded type tag
struct typed_node {
    enum node_type type;
    void *data;
    struct typed_node *next;
};

void process_users(struct typed_node *head) {
    while (head) {
        if (head->type != NODE_USER) {
            fprintf(stderr, "Unexpected node type in user list\n");
            head = head->next;
            continue;
        }
        struct user *u = (struct user *)head->data;
        grant_access(u->username, u->role);
        head = head->next;
    }
}

// Callback with typed wrapper
struct event_context {
    enum context_type type;
    void *data;
};

void on_event(void *user_data) {
    struct event_context *ctx = (struct event_context *)user_data;
    if (ctx->type != CTX_CONFIG) {
        return;
    }
    struct config *cfg = (struct config *)ctx->data;
    apply_config(cfg->settings);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `cast_expression`, `type_descriptor`, `pointer_declarator`, `identifier`
- **Detection approach**: Find `cast_expression` nodes where the target type is a pointer type (contains `*`) and the source expression is a `void*`-typed variable, function parameter, or return value. Flag casts from `void*` to struct pointer types, especially when there is no preceding conditional check (type tag validation, `assert`, or `if` guard) in the enclosing block. Higher severity when the cast target struct contains function pointers.
- **S-expression query sketch**:
```scheme
(cast_expression
  type: (type_descriptor
    declarator: (abstract_pointer_declarator))
  value: (identifier) @source)
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `void_pointer_cast`
- **Severity**: warning
- **Confidence**: high
