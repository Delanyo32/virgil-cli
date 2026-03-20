# Resource Exhaustion -- C++

## Overview
Resource exhaustion vulnerabilities in C++ arise from unbounded container growth driven by user-controlled input and stack overflow caused by unbounded recursion. C++ STL containers like `std::vector`, `std::map`, and `std::unordered_map` will dynamically grow to accommodate any number of elements, and recursive functions without depth limits can exhaust the call stack. Both patterns can be triggered by crafted input to cause denial of service.

## Why It's a Security Concern
Unbounded container growth allows attackers to exhaust heap memory by sending input that causes a container to store millions or billions of elements. `std::vector::push_back()` in a loop reading from a network socket or file can consume all available RAM, triggering the OOM killer. Stack overflow via unbounded recursion crashes the process immediately (SIGSEGV) with no opportunity for graceful recovery. In server applications, both attacks cause complete service unavailability.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: STL (vector, map, unordered_map, list, deque), Boost, standard I/O streams

---

## Pattern 1: Unbounded Container Growth from User Input

### Description
Inserting elements into an STL container (`std::vector`, `std::map`, `std::unordered_map`, `std::list`) inside a loop that reads from user-controlled input (network socket, file, stdin) without enforcing a maximum element count. The container grows without bound until memory is exhausted.

### Bad Code (Anti-pattern)
```cpp
#include <vector>
#include <string>
#include <iostream>
#include <sstream>

struct Request {
    uint32_t item_count;
    // ...
};

std::vector<std::string> readItems(std::istream& input) {
    std::vector<std::string> items;
    std::string line;
    // Reads until EOF -- no limit on number of items
    while (std::getline(input, line)) {
        items.push_back(line);
    }
    return items;
}

void processRequest(const Request& req, std::istream& input) {
    std::vector<Record> records;
    // User-controlled count drives allocation
    records.reserve(req.item_count);
    for (uint32_t i = 0; i < req.item_count; i++) {
        Record r;
        input.read(reinterpret_cast<char*>(&r), sizeof(r));
        records.push_back(r);
    }
}

void collectMetrics(int sockfd) {
    std::unordered_map<std::string, int> counters;
    char buf[4096];
    // Unbounded insertion from network data
    while (ssize_t n = recv(sockfd, buf, sizeof(buf), 0)) {
        if (n <= 0) break;
        std::string key(buf, n);
        counters[key]++;  // Grows without bound
    }
}
```

### Good Code (Fix)
```cpp
#include <vector>
#include <string>
#include <iostream>
#include <stdexcept>
#include <unordered_map>

constexpr size_t MAX_ITEMS = 100'000;
constexpr size_t MAX_UNIQUE_KEYS = 50'000;

struct Request {
    uint32_t item_count;
    // ...
};

std::vector<std::string> readItems(std::istream& input) {
    std::vector<std::string> items;
    std::string line;
    while (std::getline(input, line)) {
        if (items.size() >= MAX_ITEMS) {
            throw std::runtime_error("Too many input items");
        }
        items.push_back(std::move(line));
    }
    return items;
}

void processRequest(const Request& req, std::istream& input) {
    if (req.item_count > MAX_ITEMS) {
        throw std::runtime_error("Item count exceeds maximum");
    }
    std::vector<Record> records;
    records.reserve(req.item_count);
    for (uint32_t i = 0; i < req.item_count; i++) {
        Record r;
        if (!input.read(reinterpret_cast<char*>(&r), sizeof(r))) break;
        records.push_back(r);
    }
}

void collectMetrics(int sockfd) {
    std::unordered_map<std::string, int> counters;
    char buf[4096];
    while (ssize_t n = recv(sockfd, buf, sizeof(buf), 0)) {
        if (n <= 0) break;
        if (counters.size() >= MAX_UNIQUE_KEYS) {
            break;  // Stop accepting new keys
        }
        std::string key(buf, n);
        counters[key]++;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `while_statement`, `for_statement`, `identifier`, `template_type`
- **Detection approach**: Find `call_expression` nodes invoking `.push_back()`, `.emplace_back()`, `.insert()`, or `.emplace()` on STL containers inside `while_statement` or `for_statement` loops that read from I/O sources (`std::getline`, `recv`, `read`, `input >>`). Check the loop body for a size comparison (`items.size() >= MAX`) that would break or throw. Also find `.reserve()` calls where the argument is a struct field or variable (user-controlled capacity).
- **S-expression query sketch**:
```scheme
(while_statement
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (field_expression
          field: (field_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_container_growth`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Stack Overflow via Unbounded Recursion

### Description
Recursive functions that process user-controlled input (parsing nested structures, traversing trees, evaluating expressions) without a maximum recursion depth limit. Deeply nested input causes the call stack to overflow, crashing the process with a segmentation fault. This is especially dangerous in parsers, deserializers, and tree-walking interpreters.

### Bad Code (Anti-pattern)
```cpp
#include <string>
#include <memory>

struct JsonValue {
    std::string type;
    std::vector<std::unique_ptr<JsonValue>> children;
};

// No depth limit -- deeply nested JSON crashes the parser
JsonValue parseValue(const char*& input) {
    JsonValue val;
    if (*input == '{') {
        val.type = "object";
        input++;  // skip '{'
        while (*input != '}' && *input != '\0') {
            auto child = std::make_unique<JsonValue>(parseValue(input));
            val.children.push_back(std::move(child));
        }
        if (*input == '}') input++;
    } else if (*input == '[') {
        val.type = "array";
        input++;
        while (*input != ']' && *input != '\0') {
            auto child = std::make_unique<JsonValue>(parseValue(input));
            val.children.push_back(std::move(child));
        }
        if (*input == ']') input++;
    }
    return val;
}

// Tree traversal with no depth guard
int computeDepth(const TreeNode* node) {
    if (!node) return 0;
    int maxChild = 0;
    for (const auto& child : node->children) {
        maxChild = std::max(maxChild, computeDepth(child.get()));
    }
    return maxChild + 1;
}
```

### Good Code (Fix)
```cpp
#include <string>
#include <memory>
#include <stdexcept>

constexpr int MAX_RECURSION_DEPTH = 128;

struct JsonValue {
    std::string type;
    std::vector<std::unique_ptr<JsonValue>> children;
};

JsonValue parseValue(const char*& input, int depth = 0) {
    if (depth > MAX_RECURSION_DEPTH) {
        throw std::runtime_error("Maximum nesting depth exceeded");
    }

    JsonValue val;
    if (*input == '{') {
        val.type = "object";
        input++;
        while (*input != '}' && *input != '\0') {
            auto child = std::make_unique<JsonValue>(parseValue(input, depth + 1));
            val.children.push_back(std::move(child));
        }
        if (*input == '}') input++;
    } else if (*input == '[') {
        val.type = "array";
        input++;
        while (*input != ']' && *input != '\0') {
            auto child = std::make_unique<JsonValue>(parseValue(input, depth + 1));
            val.children.push_back(std::move(child));
        }
        if (*input == ']') input++;
    }
    return val;
}

int computeDepth(const TreeNode* node, int currentDepth = 0) {
    if (!node) return 0;
    if (currentDepth > MAX_RECURSION_DEPTH) {
        return currentDepth;  // Stop descending
    }
    int maxChild = 0;
    for (const auto& child : node->children) {
        maxChild = std::max(maxChild, computeDepth(child.get(), currentDepth + 1));
    }
    return maxChild + 1;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `call_expression`, `identifier`, `parameter_list`
- **Detection approach**: Find `function_definition` nodes where the function body contains a `call_expression` invoking the same function (direct recursion). Check whether the parameter list includes a depth counter parameter and whether the function body contains a depth comparison guard (`if (depth > MAX)`) at the entry point. Flag recursive functions that lack both a depth parameter and an early-return depth check.
- **S-expression query sketch**:
```scheme
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func_name)
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (identifier) @called_func))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_recursion_stack_overflow`
- **Severity**: warning
- **Confidence**: medium
