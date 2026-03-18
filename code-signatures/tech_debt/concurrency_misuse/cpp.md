# Concurrency Misuse -- C++

## Overview
C++ provides RAII-based concurrency primitives (`std::lock_guard`, `std::unique_lock`, `std::scoped_lock`) that automate mutex lifecycle management, yet developers frequently use manual `lock()`/`unlock()` calls that are error-prone. Additionally, detached threads that access stack-allocated variables after the launching function returns cause use-after-free bugs that are undefined behavior.

## Why It's a Tech Debt Concern
Manual `lock()`/`unlock()` on `std::mutex` bypasses RAII, the core C++ resource management idiom. Every early return, exception throw, or added branch becomes a potential deadlock when the unlock is skipped. Detached threads referencing stack variables produce memory corruption that is non-deterministic — it may work in testing but crash in production under different timing conditions, and the resulting bugs (corrupted data, segfaults) have no obvious connection to the threading code.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: C++ standard library (thread, mutex), Boost.Thread, Qt (QThread, QMutex)

---

## Pattern 1: Manual Mutex Lock/Unlock Without RAII Guard

### Description
Calling `mutex.lock()` and `mutex.unlock()` directly instead of using `std::lock_guard<std::mutex>`, `std::unique_lock<std::mutex>`, or `std::scoped_lock`. Manual locking is not exception-safe — if any code between `lock()` and `unlock()` throws, the mutex is never released, causing a deadlock.

### Bad Code (Anti-pattern)
```cpp
#include <mutex>
#include <vector>
#include <string>

class ConnectionPool {
    std::mutex mtx_;
    std::vector<Connection> pool_;

public:
    Connection acquire() {
        mtx_.lock();

        if (pool_.empty()) {
            mtx_.unlock();
            throw std::runtime_error("No connections available");  // OK here
        }

        auto conn = std::move(pool_.back());
        pool_.pop_back();

        // If pop_back() or move constructor throws, mutex is never unlocked
        mtx_.unlock();
        return conn;
    }

    void release(Connection conn) {
        mtx_.lock();
        pool_.push_back(std::move(conn));
        // If push_back() throws (allocation failure), mutex leaks
        mtx_.unlock();
    }

    size_t available() const {
        mtx_.lock();  // Error: can't lock a const mutex without mutable
        auto size = pool_.size();
        mtx_.unlock();
        return size;
    }
};
```

### Good Code (Fix)
```cpp
#include <mutex>
#include <vector>
#include <string>

class ConnectionPool {
    mutable std::mutex mtx_;
    std::vector<Connection> pool_;

public:
    Connection acquire() {
        std::lock_guard<std::mutex> lock(mtx_);

        if (pool_.empty()) {
            throw std::runtime_error("No connections available");
            // lock_guard destructor releases mutex even on throw
        }

        auto conn = std::move(pool_.back());
        pool_.pop_back();
        return conn;
        // lock_guard destructor releases mutex on normal return
    }

    void release(Connection conn) {
        std::lock_guard<std::mutex> lock(mtx_);
        pool_.push_back(std::move(conn));
        // Exception-safe: mutex released on throw or return
    }

    size_t available() const {
        std::lock_guard<std::mutex> lock(mtx_);
        return pool_.size();
    }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `function_definition`
- **Detection approach**: Find `call_expression` nodes where the callee is a `field_expression` with field name `lock` or `unlock` on an object whose type is `std::mutex` or a known mutex type. Flag when the function body contains `.lock()` and `.unlock()` calls without any `std::lock_guard`, `std::unique_lock`, or `std::scoped_lock` declarations. Check for the absence of RAII guard variable declarations in the same scope.
- **S-expression query sketch**:
```scheme
(expression_statement
  (call_expression
    function: (field_expression
      argument: (identifier) @mutex_var
      field: (field_identifier) @method_name)))

(declaration
  type: (template_type
    name: (type_identifier) @guard_type))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `manual_mutex_lock_unlock`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Detached Thread Accessing Stack Variables

### Description
A thread is created with `std::thread` and then `detach()`ed, but the thread's callable captures local variables by reference or pointer. When the launching function returns, the stack frame is destroyed, and the detached thread accesses dangling references — undefined behavior that causes memory corruption, segfaults, or silent data corruption.

### Bad Code (Anti-pattern)
```cpp
#include <thread>
#include <vector>

void processData(const std::vector<int>& input) {
    int result = 0;
    std::string status = "processing";

    std::thread worker([&result, &status, &input]() {
        // All three captures are dangling references after processData() returns
        for (int val : input) {
            result += val;
        }
        status = "done";
    });

    worker.detach();  // processData() may return before worker finishes
    // result and status destroyed here — worker now has dangling references
}

void startBackgroundTask() {
    char buffer[1024];
    snprintf(buffer, sizeof(buffer), "task data");

    std::thread([&buffer]() {
        // buffer is a stack array — dangling after startBackgroundTask() returns
        processBuffer(buffer);
    }).detach();
}
```

### Good Code (Fix)
```cpp
#include <thread>
#include <vector>
#include <memory>
#include <future>

// Option 1: Use std::async with shared ownership
std::future<int> processData(std::vector<int> input) {
    return std::async(std::launch::async, [input = std::move(input)]() {
        int result = 0;
        for (int val : input) {
            result += val;
        }
        return result;
    });
}

// Option 2: Move/copy data into the thread
void startBackgroundTask() {
    std::string data = "task data";

    std::thread([data = std::move(data)]() {
        // data is owned by the thread — safe regardless of caller lifetime
        processBuffer(data.c_str());
    }).detach();
}

// Option 3: Use join() instead of detach()
void processDataSync(const std::vector<int>& input) {
    int result = 0;
    std::thread worker([&result, &input]() {
        for (int val : input) {
            result += val;
        }
    });
    worker.join();  // Guarantees worker finishes before stack is destroyed
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `lambda_expression`, `lambda_capture_specifier`
- **Detection approach**: Find `call_expression` nodes calling `.detach()` on a `std::thread` object. Then inspect the thread constructor argument (typically a `lambda_expression`). Check the `lambda_capture_specifier` for reference captures (`&`, `&var`). Flag when any reference capture (`&`) is present in a lambda passed to a thread that is subsequently detached. Value captures (`=`, `var`) and move captures (`var = std::move(var)`) are safe.
- **S-expression query sketch**:
```scheme
(expression_statement
  (call_expression
    function: (field_expression
      argument: (identifier) @thread_var
      field: (field_identifier) @detach_method)))

(declaration
  declarator: (init_declarator
    declarator: (identifier) @thread_var
    value: (call_expression
      arguments: (argument_list
        (lambda_expression
          captures: (lambda_capture_specifier) @captures)))))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `detached_thread_dangling_ref`
- **Severity**: warning
- **Confidence**: high
