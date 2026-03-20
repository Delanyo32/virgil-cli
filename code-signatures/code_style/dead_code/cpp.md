# Dead Code -- C++

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. It also increases compilation time and complicates refactoring. In C++, dead code can lurk inside `#ifdef` blocks, unused template specializations, and anonymous namespace functions.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Function in Anonymous Namespace

### Description
A function defined inside an anonymous namespace (or declared `static`) but never called from anywhere in the translation unit. Anonymous namespaces in C++ serve the same role as `static` in C — internal linkage.

### Bad Code (Anti-pattern)
```cpp
namespace {

std::string sanitize_v1(const std::string& input) {
    std::string result;
    std::copy_if(input.begin(), input.end(), std::back_inserter(result),
                 [](char c) { return std::isalnum(c); });
    return result;
}

std::string sanitize(std::string_view input) {
    std::string result;
    result.reserve(input.size());
    for (char c : input) {
        if (std::isalnum(static_cast<unsigned char>(c))) {
            result.push_back(c);
        }
    }
    return result;
}

}  // namespace

std::string clean_input(std::string_view raw) {
    return sanitize(raw);
}
```

### Good Code (Fix)
```cpp
namespace {

std::string sanitize(std::string_view input) {
    std::string result;
    result.reserve(input.size());
    for (char c : input) {
        if (std::isalnum(static_cast<unsigned char>(c))) {
            result.push_back(c);
        }
    }
    return result;
}

}  // namespace

std::string clean_input(std::string_view raw) {
    return sanitize(raw);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` inside `namespace_definition` (anonymous) or with `static` storage class
- **Detection approach**: Collect all function definitions inside anonymous namespaces (namespaces with no name) and `static` function definitions. Cross-reference with all `call_expression` nodes in the same file. Functions with zero call sites are candidates. Exclude virtual method overrides, functions used as template arguments, functions whose addresses are taken (`&func`), and functions in header files that may be included elsewhere.
- **S-expression query sketch**:
  ```scheme
  (namespace_definition
    body: (declaration_list
      (function_definition
        declarator: (function_declarator
          declarator: (identifier) @fn_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Throw/Abort

### Description
Code statements that appear after an unconditional return, throw, `std::abort()`, `std::exit()`, `std::terminate()`, or other diverging expression — they can never execute.

### Bad Code (Anti-pattern)
```cpp
std::unique_ptr<Config> load_config(const std::filesystem::path& path) {
    if (!std::filesystem::exists(path)) {
        throw std::runtime_error("Config not found: " + path.string());
        return nullptr;  // unreachable — throw diverges
    }

    auto cfg = parse_toml(path);
    if (!cfg) {
        std::cerr << "Fatal: invalid config\n";
        std::abort();
        return nullptr;  // unreachable — abort terminates
    }

    return cfg;
}
```

### Good Code (Fix)
```cpp
std::unique_ptr<Config> load_config(const std::filesystem::path& path) {
    if (!std::filesystem::exists(path)) {
        throw std::runtime_error("Config not found: " + path.string());
    }

    auto cfg = parse_toml(path);
    if (!cfg) {
        std::cerr << "Fatal: invalid config\n";
        std::abort();
    }

    return cfg;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `throw_statement`, `call_expression` (for `std::abort`, `std::exit`, `std::terminate`, `std::_Exit`)
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `compound_statement`. In C++, `throw`, `std::abort()`, `std::exit()`, `std::terminate()`, and functions marked `[[noreturn]]` are diverging. Also check for statements after unconditional `return`. Exclude statements in `try`/`catch` blocks where the throw may be caught.
- **S-expression query sketch**:
  ```scheme
  (compound_statement
    (return_statement) @exit
    .
    (_) @unreachable)
  (compound_statement
    (throw_statement) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Dead #ifdef Blocks

### Description
Preprocessor-guarded blocks (`#ifdef`, `#if 0`) that are never compiled because the macro is never defined or the condition is always false. This is a C/C++-specific form of dead code that tree-sitter can partially detect.

### Bad Code (Anti-pattern)
```cpp
#if 0
// Old implementation using raw pointers — replaced with smart pointers in v4
class ConnectionPool {
    Connection* connections_[MAX_CONNECTIONS];
    size_t count_;

public:
    ConnectionPool() : count_(0) {
        std::memset(connections_, 0, sizeof(connections_));
    }

    ~ConnectionPool() {
        for (size_t i = 0; i < count_; ++i) {
            delete connections_[i];
        }
    }

    Connection* acquire() { /* ... */ }
    void release(Connection* conn) { /* ... */ }
};
#endif

class ConnectionPool {
    std::vector<std::unique_ptr<Connection>> connections_;

public:
    std::shared_ptr<Connection> acquire() { /* ... */ }
};
```

### Good Code (Fix)
```cpp
class ConnectionPool {
    std::vector<std::unique_ptr<Connection>> connections_;

public:
    std::shared_ptr<Connection> acquire() { /* ... */ }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_if`, `preproc_ifdef`
- **Detection approach**: Find `preproc_if` nodes with condition `0` (literal false), which are definitively dead. For `preproc_ifdef`, check if the macro name is never `#define`d in the project. Also detect `#if 0` ... `#endif` patterns that wrap large code blocks. Exclude feature-toggle macros (e.g., `DEBUG`, `NDEBUG`), platform macros (`_WIN32`, `__linux__`, `__APPLE__`), and C++ version checks (`__cplusplus >= 202002L`).
- **S-expression query sketch**:
  ```scheme
  (preproc_if
    condition: (number_literal) @cond
    (#eq? @cond "0"))
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `dead_ifdef_block`
- **Severity**: info
- **Confidence**: medium
