# Memory Leak Indicators -- C++

## Overview
Memory leaks in C++ occur through raw `new` without `delete`, `shared_ptr` reference cycles, unbounded container growth, C-style resource management (`fopen`/`fclose`), and detached threads. Modern C++ (RAII, smart pointers) mitigates many issues, but legacy patterns and misuse of smart pointers still cause leaks.

## Why It's a Scalability Concern
C++ applications are often high-performance servers, game engines, or embedded systems running continuously. Raw `new` leaks are permanent — no GC exists. `shared_ptr` cycles silently prevent deallocation. Unbounded `std::map` growth in a server handling millions of requests will consume all available memory.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: STL, Boost, Qt
- **Existing pipeline**: `shared_ptr_cycle_risk.rs` in `src/audit/pipelines/cpp/` — extends with additional patterns

---

## Pattern 1: new Without delete

### Description
Using raw `new` to allocate objects without a corresponding `delete` (or `delete[]` for arrays), and not wrapping in a smart pointer.

### Bad Code (Anti-pattern)
```cpp
void processData(const std::string& input) {
    auto* parser = new XmlParser(input);
    auto result = parser->parse();
    if (!result.isValid()) {
        return;  // early return leaks parser
    }
    use(result);
    delete parser;
}
```

### Good Code (Fix)
```cpp
void processData(const std::string& input) {
    auto parser = std::make_unique<XmlParser>(input);
    auto result = parser->parse();
    if (!result.isValid()) {
        return;  // unique_ptr automatically deletes
    }
    use(result);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `new_expression`, `declaration`, `return_statement`
- **Detection approach**: Find `new_expression` assigned to a raw pointer (not wrapped in `unique_ptr` or `shared_ptr`). Search the function for `delete` on that pointer. Flag if no `delete` exists or if there are early `return` paths before the `delete`.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var)
    value: (new_expression
      type: (type_identifier) @type)))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `new_without_delete`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: shared_ptr Cycles

### Description
Two or more objects holding `shared_ptr` references to each other, creating a reference-counting cycle that prevents either from being deallocated.

### Bad Code (Anti-pattern)
```cpp
struct Node {
    std::vector<std::shared_ptr<Node>> children;
    std::shared_ptr<Node> parent;  // cycle: parent -> child -> parent
};

auto parent = std::make_shared<Node>();
auto child = std::make_shared<Node>();
parent->children.push_back(child);
child->parent = parent;  // ref count never reaches 0
```

### Good Code (Fix)
```cpp
struct Node {
    std::vector<std::shared_ptr<Node>> children;
    std::weak_ptr<Node> parent;  // weak_ptr breaks the cycle
};

auto parent = std::make_shared<Node>();
auto child = std::make_shared<Node>();
parent->children.push_back(child);
child->parent = parent;  // weak_ptr doesn't increment ref count
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_specifier`, `field_declaration`, `template_type`, `type_identifier`
- **Detection approach**: Find `struct_specifier` (or `class_specifier`) where a field has type `shared_ptr<T>` and `T` is the same struct type or a type that also has a `shared_ptr` back-reference. Look for `shared_ptr<Node>` field inside `struct Node`.
- **S-expression query sketch**:
```scheme
(struct_specifier
  name: (type_identifier) @struct_name
  body: (field_declaration_list
    (field_declaration
      type: (template_type
        name: (type_identifier) @smart_ptr
        arguments: (template_argument_list
          (type_descriptor
            type: (type_identifier) @inner_type))))
    (#eq? @smart_ptr "shared_ptr")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `shared_ptr_cycle`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: Container Growth in Loop Without Eviction

### Description
Calling `.insert()`, `.emplace()`, or `operator[]` on `std::map`, `std::unordered_map`, or `std::vector` inside a loop without any `.erase()`, `.clear()`, or size check, causing unbounded memory growth.

### Bad Code (Anti-pattern)
```cpp
std::unordered_map<std::string, QueryResult> queryCache;

QueryResult handleQuery(const std::string& sql) {
    if (queryCache.find(sql) == queryCache.end()) {
        queryCache[sql] = executeQuery(sql);  // grows forever
    }
    return queryCache[sql];
}
```

### Good Code (Fix)
```cpp
std::unordered_map<std::string, QueryResult> queryCache;

QueryResult handleQuery(const std::string& sql) {
    if (queryCache.find(sql) == queryCache.end()) {
        if (queryCache.size() > 10000) {
            queryCache.erase(queryCache.begin());
        }
        queryCache[sql] = executeQuery(sql);
    }
    return queryCache[sql];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `subscript_expression`, `call_expression`, `field_expression`, `for_statement`
- **Detection approach**: Find `subscript_expression` (map indexing with `[]`) or `call_expression` calling `.insert()`, `.emplace()` on a container variable. Check the module/class for `.erase()`, `.clear()`, or size comparisons on the same container. Flag if no eviction exists.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (subscript_expression
    argument: (_)
    value: (identifier) @container))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `container_growth_no_eviction`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: C-style fopen Without fclose

### Description
Using C-style `fopen()` in C++ code without `fclose()`, when RAII alternatives like `std::ifstream` or `std::unique_ptr<FILE, decltype(&fclose)>` should be used instead.

### Bad Code (Anti-pattern)
```cpp
std::string readConfig(const std::string& path) {
    FILE* fp = fopen(path.c_str(), "r");
    if (!fp) return "";
    char buf[4096];
    std::string content;
    while (fgets(buf, sizeof(buf), fp)) {
        content += buf;
    }
    return content;  // fp never closed
}
```

### Good Code (Fix)
```cpp
std::string readConfig(const std::string& path) {
    std::ifstream file(path);
    if (!file.is_open()) return "";
    return std::string(std::istreambuf_iterator<char>(file),
                       std::istreambuf_iterator<char>());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `declaration`
- **Detection approach**: Find `call_expression` calling `fopen` in C++ code. Flag as a code smell suggesting RAII alternatives, and additionally check for missing `fclose`.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @var)
    value: (call_expression
      function: (identifier) @func
      (#eq? @func "fopen"))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `c_style_fopen`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 5: std::thread Without join()/detach()

### Description
Creating a `std::thread` without calling `.join()` or `.detach()` before the thread object is destroyed. The destructor calls `std::terminate()` if the thread is joinable, and leaked threads consume resources.

### Bad Code (Anti-pattern)
```cpp
void startBackgroundTask() {
    std::thread t([]() {
        heavyComputation();
    });
    // t destroyed without join or detach — std::terminate() called
}
```

### Good Code (Fix)
```cpp
void startBackgroundTask() {
    std::thread t([]() {
        heavyComputation();
    });
    t.detach();  // or use std::jthread (C++20) which auto-joins
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration`, `template_type`, `call_expression`, `field_expression`
- **Detection approach**: Find `declaration` of `std::thread` type. Search the scope for `.join()` or `.detach()` calls on that variable. Flag if neither exists before the variable goes out of scope.
- **S-expression query sketch**:
```scheme
(declaration
  type: (qualified_identifier) @type
  declarator: (init_declarator
    declarator: (identifier) @var)
  (#eq? @type "std::thread"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `thread_no_join_detach`
- **Severity**: error
- **Confidence**: high
