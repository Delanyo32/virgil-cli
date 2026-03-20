# Type Safety Gaps -- C++

## Overview
C++ inherits C-style casts that bypass the type system without any compile-time checks, and adds `reinterpret_cast` which can unsafely reinterpret memory. Modern C++ provides `static_cast`, `dynamic_cast`, and `const_cast` with well-defined semantics and compiler verification, making C-style casts and unnecessary `reinterpret_cast` usage indicators of type safety gaps.

## Why It's a Tech Debt Concern
C-style casts `(Type)expr` silently perform whichever cast is needed -- `static_cast`, `reinterpret_cast`, `const_cast`, or a combination -- without making the intent explicit. This makes code reviews harder and hides dangerous conversions (like casting away `const` or reinterpreting pointers). `reinterpret_cast` used where `static_cast` would suffice adds unnecessary risk: it performs no type checking and can produce undefined behavior if the source and target types are not layout-compatible.

## Applicability
- **Relevance**: high (C-style casts are common in legacy C++ and code ported from C)
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: All C++ codebases; Qt, Boost, game engines, embedded systems

---

## Pattern 1: C-style Casts Instead of C++ Casts

### Description
Using C-style cast syntax `(Type)expression` instead of the explicit C++ cast operators (`static_cast<Type>(expr)`, `dynamic_cast<Type>(expr)`, `const_cast<Type>(expr)`). C-style casts can perform any conversion silently, including dangerous ones like casting away `const`, reinterpreting pointer types, or truncating values, without making the programmer's intent explicit.

### Bad Code (Anti-pattern)
```cpp
void processData(const void* raw, int size) {
    char* data = (char*)raw;         // Casts away const silently
    int length = (int)size;          // Could truncate on 64-bit
    float ratio = (float)length / (float)total;  // Loses precision

    Base* base = getObject();
    Derived* derived = (Derived*)base;  // No runtime check, UB if wrong type
    derived->specificMethod();

    const Config& config = getConfig();
    Config& mutable_config = (Config&)config;  // Casts away const
    mutable_config.setValue("key", "value");

    uint64_t address = (uint64_t)ptr;  // Pointer to integer cast
}
```

### Good Code (Fix)
```cpp
void processData(const void* raw, int size) {
    const char* data = static_cast<const char*>(raw);  // Preserves const
    auto length = static_cast<size_t>(size);  // Explicit widening
    double ratio = static_cast<double>(length) / static_cast<double>(total);

    Base* base = getObject();
    Derived* derived = dynamic_cast<Derived*>(base);  // Runtime type check
    if (derived) {
        derived->specificMethod();
    }

    // Don't cast away const; redesign the API instead
    Config config = getConfig();  // Get a mutable copy
    config.setValue("key", "value");

    auto address = reinterpret_cast<uintptr_t>(ptr);  // Explicit intent
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `cast_expression`, `type_descriptor`
- **Detection approach**: Find `cast_expression` nodes, which represent C-style casts `(Type)expr` in the tree-sitter C++ grammar. These are distinct from `static_cast_expression`, `dynamic_cast_expression`, `const_cast_expression`, and `reinterpret_cast_expression` nodes which represent the C++ cast operators. Flag all `cast_expression` nodes. Optionally assess severity by checking whether the cast involves pointers, const removal, or narrowing conversions.
- **S-expression query sketch**:
```scheme
(cast_expression
  type: (type_descriptor) @cast_type
  value: (_) @value)
```

### Pipeline Mapping
- **Pipeline name**: `c_style_cast`
- **Pattern name**: `c_style_cast_usage`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: `reinterpret_cast` Usage Where `static_cast` Would Suffice

### Description
Using `reinterpret_cast` to convert between related types where `static_cast` would perform the same conversion safely. `reinterpret_cast` performs no type checking and is intended only for low-level pointer/integer reinterpretation. Using it for numeric conversions, upcasts, or `void*` conversions adds unnecessary risk and signals unclear intent.

### Bad Code (Anti-pattern)
```cpp
void process(void* data) {
    // reinterpret_cast for void* -- static_cast handles this
    auto* config = reinterpret_cast<Config*>(data);
    config->apply();
}

int convertValue(double val) {
    // reinterpret_cast for numeric conversion (actually ill-formed, but common mistake)
    return reinterpret_cast<int&>(val);
}

class Base { virtual ~Base() = default; };
class Derived : public Base {};

void upcast(Derived* d) {
    // reinterpret_cast for class hierarchy -- static_cast is correct
    Base* b = reinterpret_cast<Base*>(d);
    process(b);
}

void handleCallback(void* context) {
    // reinterpret_cast for void* round-trip -- static_cast works
    auto* handler = reinterpret_cast<EventHandler*>(context);
    handler->onEvent();
}
```

### Good Code (Fix)
```cpp
void process(void* data) {
    auto* config = static_cast<Config*>(data);
    config->apply();
}

int convertValue(double val) {
    return static_cast<int>(val);  // Explicit truncation, well-defined
}

class Base { virtual ~Base() = default; };
class Derived : public Base {};

void upcast(Derived* d) {
    Base* b = static_cast<Base*>(d);  // Implicit upcast, safe
    process(b);
}

void handleCallback(void* context) {
    auto* handler = static_cast<EventHandler*>(context);
    handler->onEvent();
}

// reinterpret_cast is appropriate here -- hardware register access
volatile uint32_t* getRegister(uintptr_t address) {
    return reinterpret_cast<volatile uint32_t*>(address);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `reinterpret_cast_expression`, `type_descriptor`, `type_identifier`
- **Detection approach**: Find `reinterpret_cast_expression` nodes. Analyze the source and target types: if the cast is from `void*` to a typed pointer, from a derived class pointer to a base class pointer, or between numeric types, flag it as a case where `static_cast` would suffice. Legitimate `reinterpret_cast` usage includes pointer-to-integer conversions, hardware register access, and casts between unrelated pointer types in serialization code.
- **S-expression query sketch**:
```scheme
(reinterpret_cast_expression
  type: (type_descriptor) @target_type
  value: (_) @source)
```

### Pipeline Mapping
- **Pipeline name**: `c_style_cast`
- **Pattern name**: `unnecessary_reinterpret_cast`
- **Severity**: warning
- **Confidence**: medium
