# Type Confusion -- C++

## Overview
C++ provides four explicit cast operators with different safety guarantees: `static_cast`, `dynamic_cast`, `reinterpret_cast`, and `const_cast`. Type confusion vulnerabilities arise when `reinterpret_cast` is used between unrelated types (reinterpreting raw memory) or when `static_cast` is used for downcasting polymorphic types instead of `dynamic_cast` (bypassing runtime type checking). Both patterns can lead to undefined behavior, memory corruption, and exploitable crashes.

## Why It's a Security Concern
`reinterpret_cast` performs no validation -- it reinterprets the bit pattern of one type as another, similar to C's pointer cast but explicit. Using it between unrelated types produces undefined behavior and can corrupt vtable pointers, enabling vtable hijacking attacks. `static_cast` for polymorphic downcasts skips the RTTI check that `dynamic_cast` performs -- if the object is not actually the target type, subsequent virtual method calls dispatch through a wrong vtable, which attackers can exploit for code execution. These patterns are common exploitation targets in browsers (V8, WebKit), media parsers, and game engines.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: Chromium, LLVM, Qt, Boost, game engines (Unreal, Godot), any C++ codebase using polymorphism or type erasure

---

## Pattern 1: reinterpret_cast Between Unrelated Types

### Description
Using `reinterpret_cast` to convert between pointer types that do not share a common base class or compatible memory layout. Unlike `static_cast`, `reinterpret_cast` performs zero type checking -- it simply reinterprets the pointer's bit pattern. This is valid only for round-trip conversions (e.g., `T* -> void* -> T*`) and between pointer and integer types. Using it between unrelated struct/class types accesses memory with wrong layout assumptions.

### Bad Code (Anti-pattern)
```cpp
struct NetworkHeader {
    uint32_t magic;
    uint32_t length;
};

struct CommandPayload {
    uint32_t opcode;
    char data[256];
    void (*handler)(CommandPayload*);
};

void processPacket(const uint8_t* buffer, size_t len) {
    // Reinterprets raw bytes as a struct -- no alignment or size validation
    auto* header = reinterpret_cast<const NetworkHeader*>(buffer);

    // Worse: reinterprets one struct type as a completely different one
    auto* cmd = reinterpret_cast<CommandPayload*>(
        const_cast<uint8_t*>(buffer + sizeof(NetworkHeader)));
    cmd->handler(cmd);  // calls function pointer from attacker-controlled data
}

class Base {
    virtual void process();
};

class Derived : public Base {
    int sensitiveData;
    void process() override;
};

class Unrelated {
    char buffer[128];
};

void confuseTypes(Unrelated* obj) {
    // reinterpret_cast between unrelated class hierarchies
    auto* derived = reinterpret_cast<Derived*>(obj);
    derived->process();  // vtable corruption -- undefined behavior
}
```

### Good Code (Fix)
```cpp
struct NetworkHeader {
    uint32_t magic;
    uint32_t length;
};

struct CommandPayload {
    uint32_t opcode;
    char data[256];
};

void processPacket(const uint8_t* buffer, size_t len) {
    if (len < sizeof(NetworkHeader)) return;

    // Copy to properly aligned struct
    NetworkHeader header;
    std::memcpy(&header, buffer, sizeof(header));

    if (len < sizeof(NetworkHeader) + sizeof(CommandPayload)) return;

    CommandPayload cmd;
    std::memcpy(&cmd, buffer + sizeof(NetworkHeader), sizeof(cmd));

    // Use a dispatch table instead of function pointers in data
    dispatchCommand(cmd.opcode, cmd.data);
}

void safeDowncast(Base* obj) {
    // Use dynamic_cast for polymorphic types
    auto* derived = dynamic_cast<Derived*>(obj);
    if (derived) {
        derived->process();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `template_function`, `template_argument_list`, `type_descriptor`
- **Detection approach**: Find `call_expression` nodes where the function is `reinterpret_cast` with a `template_argument_list` specifying the target type. Flag all `reinterpret_cast` usage between pointer types where the source and target are not `void*`, `char*`, `uint8_t*`, or integer types (the legitimate use cases). Higher severity when the target type contains virtual methods or function pointers.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (template_function
    name: (identifier) @cast_kind
    arguments: (template_argument_list
      (type_descriptor) @target_type))
  (#eq? @cast_kind "reinterpret_cast"))
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `reinterpret_cast_unrelated`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: static_cast Downcast Without dynamic_cast Check

### Description
Using `static_cast` to downcast a base class pointer/reference to a derived class without verifying the runtime type using `dynamic_cast`. `static_cast` performs no runtime type checking for polymorphic downcasts -- it trusts the programmer that the object is the claimed type. If the object is actually a different derived class or the base class itself, the cast produces a pointer to an object of the wrong type, and subsequent member accesses or virtual calls cause undefined behavior.

### Bad Code (Anti-pattern)
```cpp
class Shape {
public:
    virtual double area() = 0;
    virtual ~Shape() = default;
};

class Circle : public Shape {
public:
    double radius;
    double area() override { return 3.14159 * radius * radius; }
    double circumference() { return 2 * 3.14159 * radius; }
};

class Rectangle : public Shape {
public:
    double width, height;
    double area() override { return width * height; }
};

void processShape(Shape* shape) {
    // static_cast does not verify runtime type
    // If shape is actually a Rectangle, this is undefined behavior
    auto* circle = static_cast<Circle*>(shape);
    double circ = circle->circumference();  // accesses wrong memory
    logMetric("circumference", circ);
}

void handleEvent(BaseEvent* event) {
    // Assumes event type based on external tag without verification
    auto* click = static_cast<ClickEvent*>(event);
    processClick(click->x, click->y);  // UB if event is KeyEvent
}
```

### Good Code (Fix)
```cpp
void processShape(Shape* shape) {
    // dynamic_cast returns nullptr for pointer casts on type mismatch
    auto* circle = dynamic_cast<Circle*>(shape);
    if (circle) {
        double circ = circle->circumference();
        logMetric("circumference", circ);
    }
}

void handleEvent(BaseEvent* event) {
    // Use dynamic_cast for safe downcasting
    if (auto* click = dynamic_cast<ClickEvent*>(event)) {
        processClick(click->x, click->y);
    } else if (auto* key = dynamic_cast<KeyEvent*>(event)) {
        processKey(key->keyCode);
    } else {
        logWarning("Unknown event type");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `template_function`, `template_argument_list`, `type_descriptor`, `pointer_declarator`
- **Detection approach**: Find `call_expression` nodes where the function is `static_cast` with a `template_argument_list` specifying a pointer-to-derived-class type. Determine whether the source expression is a pointer-to-base-class (requires type analysis or naming conventions). Flag when the target type is a class with virtual methods (suggesting polymorphic hierarchy where `dynamic_cast` should be used). Exclude `static_cast` for numeric conversions and non-polymorphic types.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (template_function
    name: (identifier) @cast_kind
    arguments: (template_argument_list
      (type_descriptor
        declarator: (abstract_pointer_declarator)) @target_type))
  (#eq? @cast_kind "static_cast"))
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `static_cast_downcast`
- **Severity**: warning
- **Confidence**: medium
