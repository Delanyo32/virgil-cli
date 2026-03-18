# Path Traversal -- C++

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the system by crafting paths that escape the intended base directory. C++ programs often handle high-trust operations, and path traversal can lead to source code disclosure, configuration file leaks, or remote code execution.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: std::filesystem, fstream, Boost.Filesystem, POSIX

---

## Pattern 1: User Input in File Path

### Description
Using `std::filesystem::path(base) / userInput` or string concatenation to build a file path from user-supplied input without resolving the canonical path via `.canonical()` or `std::filesystem::canonical()` and verifying it starts with the intended base directory.

### Bad Code (Anti-pattern)
```cpp
#include <filesystem>
#include <fstream>
#include <string>

std::string serve_file(const std::string& base, const std::string& user_input) {
    std::filesystem::path file_path = std::filesystem::path(base) / user_input;
    std::ifstream ifs(file_path);
    return std::string(std::istreambuf_iterator<char>(ifs),
                       std::istreambuf_iterator<char>());
}
```

### Good Code (Fix)
```cpp
#include <filesystem>
#include <fstream>
#include <string>
#include <stdexcept>

std::string serve_file(const std::string& base, const std::string& user_input) {
    auto base_dir = std::filesystem::canonical(base);
    auto file_path = std::filesystem::canonical(base_dir / user_input);
    // Verify the resolved path is still under the base directory
    auto [base_end, _] = std::mismatch(base_dir.begin(), base_dir.end(), file_path.begin());
    if (base_end != base_dir.end()) {
        throw std::runtime_error("Access denied: path escapes base directory");
    }
    std::ifstream ifs(file_path);
    return std::string(std::istreambuf_iterator<char>(ifs),
                       std::istreambuf_iterator<char>());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `binary_expression`, `qualified_identifier`, `identifier`, `field_expression`
- **Detection approach**: Find expressions using the `/` operator on `std::filesystem::path` objects or `call_expression` nodes constructing paths with user-supplied arguments. Flag when the result is passed to `std::ifstream`, `std::ofstream`, `fopen()`, or similar without a preceding `std::filesystem::canonical()` call and prefix verification.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: "/"
  left: (call_expression
    function: (qualified_identifier) @path_ctor)
  right: (identifier) @user_input)
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `user_input_in_file_path`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Directory Traversal via ../

### Description
Accepting file paths that contain `../` or `..\\` sequences without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```cpp
#include <fstream>
#include <string>

std::string read_upload(const std::string& filename) {
    // No check for ".." — attacker sends "../../etc/passwd"
    std::string path = "./uploads/" + filename;
    std::ifstream ifs(path);
    return std::string(std::istreambuf_iterator<char>(ifs),
                       std::istreambuf_iterator<char>());
}
```

### Good Code (Fix)
```cpp
#include <filesystem>
#include <fstream>
#include <string>
#include <stdexcept>

std::string read_upload(const std::string& filename) {
    if (filename.find("..") != std::string::npos) {
        throw std::invalid_argument("Invalid filename");
    }
    auto base_dir = std::filesystem::canonical("./uploads");
    auto file_path = std::filesystem::canonical(base_dir / filename);
    // Double-check with canonical path prefix verification
    auto base_str = base_dir.string();
    auto file_str = file_path.string();
    if (file_str.compare(0, base_str.size(), base_str) != 0) {
        throw std::runtime_error("Path escapes base directory");
    }
    std::ifstream ifs(file_path);
    return std::string(std::istreambuf_iterator<char>(ifs),
                       std::istreambuf_iterator<char>());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `binary_expression`, `string_literal`, `identifier`, `declaration`
- **Detection approach**: Find `declaration` nodes where a `std::string` or `std::filesystem::path` is initialized with string concatenation (`+` operator) involving a path prefix and user-supplied variable, then passed to `std::ifstream`, `std::ofstream`, or `fopen()`. Flag when there is no preceding check for `".."` via `.find("..")` or `std::filesystem::canonical()` + prefix verification.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    value: (binary_expression
      operator: "+"
      left: (string_literal) @path_prefix
      right: (identifier) @user_var)))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
