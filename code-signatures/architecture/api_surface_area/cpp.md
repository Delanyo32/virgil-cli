# API Surface Area -- C++

## Overview
API surface area in C++ encompasses all publicly accessible symbols in headers: public class members, free functions, and namespace-level declarations. C++ offers fine-grained visibility control through `public`, `protected`, and `private` access specifiers within classes, but poor discipline leads to oversized public interfaces. Tracking the ratio of public to total symbols helps identify modules that expose too much, creating fragile dependencies across the codebase.

## Why It's an Architecture Concern
Large public APIs in C++ amplify compilation coupling through headers — every public member change triggers recompilation of all dependents. Excessive public interfaces also make it difficult to maintain ABI stability in shared libraries and increase the cognitive load on consumers who must navigate many entry points. When classes expose data members directly instead of through accessors, internal representation changes ripple outward. Keeping public surfaces narrow through the Pimpl idiom, private members, and internal namespaces limits blast radius and preserves freedom to refactor.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.cpp, .cc, .cxx, .hpp, .hxx, .hh`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```cpp
class DataProcessor {
public:
    void loadFromFile(const std::string& path);
    void loadFromStream(std::istream& stream);
    void validate();
    void normalizeFields();
    void deduplicateRows();
    void sortByKey(const std::string& key);
    void filterNulls();
    void applyTransform(TransformFn fn);
    void writeToFile(const std::string& path);
    void writeToStream(std::ostream& stream);
    void printSummary() const;
    std::vector<Row> getRows() const;
    std::map<std::string, int> getStats() const;
};
```

### Good Code (Fix)
```cpp
class DataProcessor {
public:
    void load(const std::string& path);
    void process();
    void save(const std::string& path);
    void printSummary() const;

private:
    void loadFromStream(std::istream& stream);
    void validate();
    void normalizeFields();
    void deduplicateRows();
    void sortByKey(const std::string& key);
    void filterNulls();
    void applyTransform(TransformFn fn);
    void writeToStream(std::ostream& stream);
    std::vector<Row> rows_;
    std::map<std::string, int> stats_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `declaration` inside `access_specifier` blocks
- **Detection approach**: Within class definitions, count members under each access specifier. Members under `public:` are exported. Flag classes where total members >= 10 and public/total > 0.8. Also count namespace-level non-static free functions.
- **S-expression query sketch**:
```scheme
;; Match public access specifier sections
(access_specifier) @access

;; Match function declarations inside class bodies
(class_specifier
  name: (type_identifier) @class.name
  body: (field_declaration_list
    (function_definition
      declarator: (function_declarator
        declarator: (field_identifier) @method.name))))

;; Match public method declarations
(class_specifier
  body: (field_declaration_list
    (declaration
      declarator: (function_declarator
        declarator: (field_identifier) @public.method.name))))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```cpp
class HttpClient {
public:
    std::string base_url;
    std::map<std::string, std::string> headers;
    std::vector<std::string> retry_log;
    int timeout_ms;
    int max_retries;
    bool verify_ssl;

    HttpResponse get(const std::string& path);
    HttpResponse post(const std::string& path, const std::string& body);
};
```

### Good Code (Fix)
```cpp
class HttpClient {
public:
    explicit HttpClient(const HttpClientConfig& config);
    HttpResponse get(const std::string& path);
    HttpResponse post(const std::string& path, const std::string& body);

    void setTimeout(int ms);
    int timeout() const;

private:
    std::string base_url_;
    std::map<std::string, std::string> headers_;
    std::vector<std::string> retry_log_;
    int timeout_ms_;
    int max_retries_;
    bool verify_ssl_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` inside `class_specifier` under `public` access
- **Detection approach**: Find classes with public data members (field declarations that are not function declarators). Flag classes where public non-method members exist alongside public methods, indicating leaked implementation details.
- **S-expression query sketch**:
```scheme
;; Match public data members (non-function fields)
(class_specifier
  name: (type_identifier) @class.name
  body: (field_declaration_list
    (field_declaration
      type: (_) @field.type
      declarator: (field_identifier) @field.name)))

;; Distinguish from function declarations by absence of function_declarator
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
