# Legacy API Usage -- C++

## Overview
Legacy API usage in C++ refers to relying on older idioms or suboptimal standard library usage when modern C++ provides safer, faster, or more expressive alternatives. Common examples include using `std::endl` instead of `'\n'` for line breaks, including entire large headers when only a single declaration is needed, and omitting the `override` keyword on virtual method overrides.

## Why It's a Tech Debt Concern
`std::endl` flushes the stream buffer on every call, causing significant I/O performance degradation in loops -- up to 10x slower than `'\n'`. Excessive includes increase compilation times, create unnecessary coupling between translation units, and slow down incremental builds. Missing `override` keywords allow silent bugs where a method signature mismatch (typo, wrong parameter type) creates a new virtual method instead of overriding the intended one, and the compiler cannot catch this without `override`. All three patterns compound as the codebase grows.

## Applicability
- **Relevance**: high (all three patterns are ubiquitous in C++ codebases of all sizes)
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: endl Instead of '\n'

### Description
Using `std::endl` to insert a newline character when a simple `'\n'` suffices. `std::endl` writes `'\n'` and then flushes the stream buffer (`std::flush`). In most cases the flush is unnecessary and dramatically slows down output, especially in loops or high-throughput logging.

### Bad Code (Anti-pattern)
```cpp
#include <iostream>
#include <vector>
#include <string>

void printReport(const std::vector<Record>& records) {
    std::cout << "=== Report ===" << std::endl;
    std::cout << "Total records: " << records.size() << std::endl;
    std::cout << std::endl;

    for (const auto& record : records) {
        std::cout << "ID: " << record.id << std::endl;
        std::cout << "Name: " << record.name << std::endl;
        std::cout << "Value: " << record.value << std::endl;
        std::cout << "---" << std::endl;
    }

    std::cout << "=== End Report ===" << std::endl;
}

void writeLog(std::ofstream& logFile, const std::vector<std::string>& entries) {
    for (const auto& entry : entries) {
        logFile << "[" << timestamp() << "] " << entry << std::endl;
        // Each endl forces a disk write -- extremely slow with thousands of entries
    }
}
```

### Good Code (Fix)
```cpp
#include <iostream>
#include <vector>
#include <string>

void printReport(const std::vector<Record>& records) {
    std::cout << "=== Report ===\n";
    std::cout << "Total records: " << records.size() << '\n';
    std::cout << '\n';

    for (const auto& record : records) {
        std::cout << "ID: " << record.id << '\n';
        std::cout << "Name: " << record.name << '\n';
        std::cout << "Value: " << record.value << '\n';
        std::cout << "---\n";
    }

    std::cout << "=== End Report ===\n";
    // Flush once at the end if needed
    std::cout.flush();
}

void writeLog(std::ofstream& logFile, const std::vector<std::string>& entries) {
    for (const auto& entry : entries) {
        logFile << '[' << timestamp() << "] " << entry << '\n';
    }
    logFile.flush();  // Single flush after all writes
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `identifier` with value `endl`, `qualified_identifier` with `std::endl`
- **Detection approach**: Find `identifier` nodes with value `endl` or `qualified_identifier` nodes matching `std::endl` used as arguments to the `<<` stream insertion operator. Flag every occurrence. Higher severity when the `endl` appears inside a loop body (`for_statement`, `while_statement`, `for_range_loop`).
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: "<<"
  right: (qualified_identifier
    scope: (namespace_identifier) @ns
    name: (identifier) @name)
  (#eq? @ns "std")
  (#eq? @name "endl"))

(binary_expression
  operator: "<<"
  right: (identifier) @name
  (#eq? @name "endl"))
```

### Pipeline Mapping
- **Pipeline name**: `endl_flush`
- **Pattern name**: `endl_instead_of_newline`
- **Severity**: info
- **Confidence**: high

---

## Pattern 2: Excessive Includes

### Description
Including large standard library or project headers (e.g., `<algorithm>`, `<iostream>`, `<vector>`, or monolithic project headers) when only a single class, function, or type from that header is used. This inflates compilation times, creates unnecessary translation unit dependencies, and slows incremental rebuilds.

### Bad Code (Anti-pattern)
```cpp
// utils.hpp -- includes everything "just in case"
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <functional>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <regex>       // One of the heaviest STL headers
#include <sstream>
#include <string>
#include <thread>
#include <unordered_map>
#include <variant>
#include <vector>

// Only uses std::string and std::vector
class Config {
public:
    std::string name;
    std::vector<std::string> values;
    std::string get(const std::string& key) const;
};
```

### Good Code (Fix)
```cpp
// utils.hpp -- include only what you use
#include <string>
#include <vector>

class Config {
public:
    std::string name;
    std::vector<std::string> values;
    std::string get(const std::string& key) const;
};

// If a type is only used as a pointer/reference in the header, forward-declare it
// #include "Database.hpp"  // Don't include if only used as pointer
class Database;  // Forward declaration suffices

class Repository {
public:
    void setDatabase(Database* db);
private:
    Database* db_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`, `system_lib_string`, `string_literal`
- **Detection approach**: Count `preproc_include` directives in a file. Flag files with more than 15 includes. Cross-reference included headers against actual usage by scanning for identifiers from each header's namespace. Detect forward-declarable types: if a type from an included header is only used as a pointer or reference in the current file, suggest a forward declaration instead.
- **S-expression query sketch**:
```scheme
(preproc_include
  path: (system_lib_string) @header)

(preproc_include
  path: (string_literal) @header)
```

### Pipeline Mapping
- **Pipeline name**: `excessive_includes`
- **Pattern name**: `unnecessary_header_inclusion`
- **Severity**: info
- **Confidence**: low

---

## Pattern 3: Missing override Keyword

### Description
Overriding a virtual method in a derived class without using the `override` keyword (available since C++11). Without `override`, a signature mismatch (typo in method name, wrong parameter type, missing `const`) silently creates a new virtual method instead of overriding the base class method. The compiler cannot detect this error without the `override` specifier.

### Bad Code (Anti-pattern)
```cpp
class Shape {
public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
    virtual double perimeter() const = 0;
    virtual std::string describe() const { return "shape"; }
    virtual void draw(Canvas& canvas) const = 0;
    virtual bool contains(double x, double y) const = 0;
};

class Circle : public Shape {
public:
    Circle(double radius) : radius_(radius) {}

    // Missing override -- if base signature changes, this silently becomes a new method
    double area() const { return 3.14159 * radius_ * radius_; }
    double perimeter() const { return 2 * 3.14159 * radius_; }
    std::string describe() const { return "circle"; }
    void draw(Canvas& canvas) const { canvas.drawCircle(0, 0, radius_); }

    // Bug: parameter mismatch -- this is a NEW method, not an override
    // Base has (double x, double y), this has (int x, int y)
    bool contains(int x, int y) const { return x*x + y*y <= radius_*radius_; }

private:
    double radius_;
};
```

### Good Code (Fix)
```cpp
class Shape {
public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
    virtual double perimeter() const = 0;
    virtual std::string describe() const { return "shape"; }
    virtual void draw(Canvas& canvas) const = 0;
    virtual bool contains(double x, double y) const = 0;
};

class Circle : public Shape {
public:
    Circle(double radius) : radius_(radius) {}

    double area() const override { return 3.14159 * radius_ * radius_; }
    double perimeter() const override { return 2 * 3.14159 * radius_; }
    std::string describe() const override { return "circle"; }
    void draw(Canvas& canvas) const override { canvas.drawCircle(0, 0, radius_); }

    // Compiler error with override: no matching base method with (int, int) signature
    // bool contains(int x, int y) const override { ... }  // ERROR
    bool contains(double x, double y) const override {
        return x*x + y*y <= radius_*radius_;
    }

private:
    double radius_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` inside a derived class (class with `base_class_clause`), `virtual_specifier`
- **Detection approach**: Find `function_definition` nodes inside `class_specifier` bodies that have a `base_class_clause` (i.e., derived classes). Check whether the function has a `virtual_specifier` child containing `override`. Flag methods in derived classes that match a naming pattern of known virtual methods from the base class but lack `override`. Heuristic: any non-static, non-constructor method in a derived class that does not have `override` is suspect.
- **S-expression query sketch**:
```scheme
(class_specifier
  name: (type_identifier) @class_name
  (base_class_clause) @bases
  body: (field_declaration_list
    (function_definition
      declarator: (function_declarator
        declarator: (field_identifier) @method_name)
      (#not-match? @method_name "^(operator|~)")
      )))
```

### Pipeline Mapping
- **Pipeline name**: `missing_override`
- **Pattern name**: `virtual_override_without_keyword`
- **Severity**: warning
- **Confidence**: medium
