# Sync Blocking in Async -- C++

## Overview
Synchronous blocking in C++ async contexts occurs when blocking I/O, `std::this_thread::sleep_for()`, or blocking system calls are used inside C++20 coroutines (`co_await`/`co_return`) or Boost.Asio coroutines, stalling the coroutine scheduler's thread.

## Why It's a Scalability Concern
C++20 coroutines and Asio coroutines are designed to multiplex many tasks on a small thread pool. A blocking call inside a coroutine holds its executor thread, preventing other coroutines from being resumed. With a pool of N threads and N blocking coroutines, the entire event loop stalls.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: C++20 coroutines, Boost.Asio, cppcoro, folly::coro

---

## Pattern 1: Blocking I/O in co_await Coroutine

### Description
Using blocking I/O functions like `std::ifstream`, `read()`, `recv()` inside a C++20 coroutine body (function containing `co_await` or `co_return`), blocking the executor thread.

### Bad Code (Anti-pattern)
```cpp
task<std::string> read_config() {
    std::ifstream file("/etc/app/config.json");
    std::string content((std::istreambuf_iterator<char>(file)),
                         std::istreambuf_iterator<char>());
    co_return content;
}
```

### Good Code (Fix)
```cpp
asio::awaitable<std::string> read_config(asio::io_context& ctx) {
    auto file = co_await asio::async_file::open(ctx, "/etc/app/config.json");
    std::string content;
    co_await asio::async_read(file, asio::dynamic_buffer(content));
    co_return content;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `co_return_statement`, `co_await_expression`, `declaration`
- **Detection approach**: Identify functions containing `co_await_expression` or `co_return_statement` (making them coroutines). Within those functions, find usage of blocking I/O types like `std::ifstream`, `std::ofstream`, or blocking function calls like `read()`, `recv()`, `fread()`.
- **S-expression query sketch**:
```scheme
(function_definition
  body: (compound_statement
    (declaration
      type: (qualified_identifier) @type_name)
    (co_return_statement)))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_io_in_coroutine`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: std::this_thread::sleep_for in Coroutine

### Description
Using `std::this_thread::sleep_for()` inside a coroutine, blocking the executor thread instead of using an async timer.

### Bad Code (Anti-pattern)
```cpp
task<void> retry_operation() {
    for (int i = 0; i < 3; ++i) {
        try {
            co_await do_operation();
            co_return;
        } catch (...) {
            std::this_thread::sleep_for(std::chrono::seconds(1));
        }
    }
    throw std::runtime_error("all retries failed");
}
```

### Good Code (Fix)
```cpp
asio::awaitable<void> retry_operation(asio::io_context& ctx) {
    for (int i = 0; i < 3; ++i) {
        try {
            co_await do_operation();
            co_return;
        } catch (...) {
            asio::steady_timer timer(ctx, std::chrono::seconds(1));
            co_await timer.async_wait(asio::use_awaitable);
        }
    }
    throw std::runtime_error("all retries failed");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `call_expression`, `qualified_identifier`, `co_await_expression`
- **Detection approach**: Find `call_expression` calling `std::this_thread::sleep_for` inside a function body that also contains `co_await_expression` or `co_return_statement`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (qualified_identifier) @func_path
  (#match? @func_path "this_thread.*sleep"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `thread_sleep_in_coroutine`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Blocking stdin/getline in ASIO Coroutine

### Description
Using `std::cin >> var`, `std::getline(std::cin, ...)`, or `scanf()` inside an Asio or C++20 coroutine, blocking the I/O thread waiting for console input.

### Bad Code (Anti-pattern)
```cpp
asio::awaitable<void> interactive_session(tcp::socket socket) {
    std::string line;
    while (true) {
        std::getline(std::cin, line); // blocks the io_context thread
        co_await asio::async_write(socket, asio::buffer(line + "\n"), asio::use_awaitable);
    }
}
```

### Good Code (Fix)
```cpp
asio::awaitable<void> interactive_session(tcp::socket socket, asio::posix::stream_descriptor& input) {
    std::string line;
    asio::streambuf buf;
    while (true) {
        co_await asio::async_read_until(input, buf, '\n', asio::use_awaitable);
        std::istream is(&buf);
        std::getline(is, line);
        co_await asio::async_write(socket, asio::buffer(line + "\n"), asio::use_awaitable);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `qualified_identifier`, `co_await_expression`
- **Detection approach**: Find `call_expression` calling `std::getline` with `std::cin` argument, or `operator>>` with `std::cin` as left operand, inside a function containing `co_await_expression`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (qualified_identifier) @func_name
  arguments: (argument_list
    (qualified_identifier) @arg)
  (#eq? @func_name "std::getline")
  (#eq? @arg "std::cin"))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_stdin_in_coroutine`
- **Severity**: warning
- **Confidence**: high
