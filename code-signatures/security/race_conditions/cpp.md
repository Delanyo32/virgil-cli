# Race Conditions -- C++

## Overview
C++ supports multi-threading natively since C++11 with `std::thread`, `std::mutex`, and `std::atomic`, but shared mutable state accessed without proper synchronization remains a common source of data races. Additionally, TOCTOU vulnerabilities in filesystem operations -- checking file properties with `std::filesystem::exists()` before operating on the file -- follow the same pattern as in C but with C++ standard library APIs. Both categories produce undefined behavior per the C++ memory model.

## Why It's a Security Concern
Data races in C++ are undefined behavior per the C++ standard (and not merely implementation-defined), meaning the compiler is free to optimize in ways that produce arbitrarily incorrect results, including security-critical logic. In practice, data races lead to corrupted data structures, use-after-free conditions, type confusion, and exploitable memory safety violations. TOCTOU races in filesystem operations enable the same symlink and file replacement attacks as in C, and are especially dangerous in privileged system services written in C++.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: std::thread, std::mutex, std::atomic, std::filesystem, Boost.Thread, Boost.Filesystem, POSIX threads

---

## Pattern 1: Data Race on Shared State Without Mutex

### Description
Accessing a shared variable (global, static, or member variable) from multiple `std::thread` instances without protecting the access with `std::mutex`, `std::lock_guard`, `std::unique_lock`, or `std::atomic`. Common patterns include incrementing shared counters, modifying shared containers (which are never thread-safe in the C++ standard library), and reading/writing shared flags used for inter-thread coordination.

### Bad Code (Anti-pattern)
```cpp
#include <thread>
#include <vector>

int counter = 0;

void worker(int iterations) {
    for (int i = 0; i < iterations; ++i) {
        // DATA RACE: read-modify-write without synchronization
        counter++;
    }
}

int main() {
    std::vector<std::thread> threads;
    for (int i = 0; i < 4; ++i) {
        threads.emplace_back(worker, 100000);
    }
    for (auto& t : threads) {
        t.join();
    }
    // counter is likely much less than 400000
    return 0;
}
```

### Good Code (Fix)
```cpp
#include <thread>
#include <vector>
#include <atomic>

std::atomic<int> counter{0};

void worker(int iterations) {
    for (int i = 0; i < iterations; ++i) {
        counter.fetch_add(1, std::memory_order_relaxed);
    }
}

int main() {
    std::vector<std::thread> threads;
    for (int i = 0; i < 4; ++i) {
        threads.emplace_back(worker, 100000);
    }
    for (auto& t : threads) {
        t.join();
    }
    // counter is exactly 400000
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `function_definition`, `update_expression`, `compound_assignment_expression`, `identifier`, `namespace_identifier`
- **Detection approach**: Find global or static variable declarations followed by `update_expression` (`counter++`) or `compound_assignment_expression` (`counter += 1`) on those variables inside functions that are passed to `std::thread` constructors. Flag when the variable is not `std::atomic<>` and the modification is not inside a `std::lock_guard`/`std::unique_lock` scope or between `mutex.lock()`/`mutex.unlock()` calls.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (compound_statement
    (expression_statement
      (update_expression
        argument: (identifier) @shared_var))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `shared_state_data_race`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: TOCTOU in Filesystem Operations

### Description
Using `std::filesystem::exists()`, `std::filesystem::is_regular_file()`, or `std::filesystem::status()` to check file properties before calling `std::filesystem::create_directory()`, `std::ofstream` open, `std::filesystem::remove()`, or similar operations. Between the check and the operation, another process or thread can alter the filesystem state, leading to race conditions exploitable via symlinks or file replacement.

### Bad Code (Anti-pattern)
```cpp
#include <filesystem>
#include <fstream>

namespace fs = std::filesystem;

void writeIfAbsent(const fs::path& path, const std::string& data) {
    if (!fs::exists(path)) {
        // RACE: file or symlink can be created between check and write
        std::ofstream out(path);
        out << data;
    }
}
```

### Good Code (Fix)
```cpp
#include <filesystem>
#include <fstream>
#include <fcntl.h>
#include <unistd.h>

namespace fs = std::filesystem;

void writeIfAbsent(const fs::path& path, const std::string& data) {
    // Use O_CREAT | O_EXCL for atomic create-or-fail (POSIX)
    int fd = open(path.c_str(), O_WRONLY | O_CREAT | O_EXCL, 0644);
    if (fd < 0) {
        if (errno == EEXIST) {
            return;  // file already exists -- safe to skip
        }
        throw std::system_error(errno, std::system_category(), "open");
    }
    ssize_t written = write(fd, data.c_str(), data.size());
    close(fd);
    if (written < 0) {
        throw std::system_error(errno, std::system_category(), "write");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `qualified_identifier`, `namespace_identifier`, `if_statement`, `identifier`
- **Detection approach**: Find `if_statement` nodes whose condition contains a `call_expression` invoking `std::filesystem::exists`, `fs::exists`, `std::filesystem::is_regular_file`, or similar check functions, where the body contains a file-mutating operation (constructing `std::ofstream`, calling `fs::remove`, `fs::create_directory`, `fs::rename`, etc.) on the same path. The non-atomic check-then-act pattern is the indicator.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (call_expression
    function: (qualified_identifier
      scope: (namespace_identifier) @ns
      name: (identifier) @method)
    (#eq? @method "exists"))
  consequence: (compound_statement
    (declaration
      declarator: (init_declarator
        value: (call_expression
          function: (_) @action_func)))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `filesystem_toctou`
- **Severity**: warning
- **Confidence**: high
