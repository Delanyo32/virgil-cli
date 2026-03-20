# Injection -- C++

## Overview
Injection vulnerabilities in C++ combine the risks inherited from C's standard library (`system()`, `printf`) with additional attack surfaces from C++ string manipulation and ORM/database libraries. The ease of building command strings with `std::string` concatenation and `+` operator, combined with direct system call APIs, creates injection vectors when user input is not validated or parameterized.

## Why It's a Security Concern
Command injection through `system()` or `popen()` with user-controlled `std::string` arguments allows arbitrary OS command execution. SQL injection via string concatenation in database query builders bypasses parameterized query protection. C++ applications often run as performance-critical services with elevated privileges, making successful injection attacks especially damaging.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: cstdlib (system), SOCI, OTL, Qt SQL, libpqxx, mysql-connector-cpp

---

## Pattern 1: Command Injection via system() with User-Controlled Strings

### Description
Building command strings using `std::string` concatenation (`+` operator or `append()`) with user-supplied input and passing the result to `system()` or `popen()`. Unlike `execvp()` which takes an argument array, `system()` invokes a shell that interprets metacharacters.

### Bad Code (Anti-pattern)
```cpp
#include <cstdlib>
#include <string>

void processFile(const std::string& filename) {
    std::string cmd = "convert " + filename + " output.png";
    system(cmd.c_str());
}

std::string getDiskUsage(const std::string& path) {
    std::string cmd = "du -sh " + path;
    FILE* pipe = popen(cmd.c_str(), "r");
    char buffer[128];
    std::string result;
    while (fgets(buffer, sizeof(buffer), pipe)) {
        result += buffer;
    }
    pclose(pipe);
    return result;
}
```

### Good Code (Fix)
```cpp
#include <string>
#include <array>
#include <memory>
#include <stdexcept>
#include <unistd.h>
#include <sys/wait.h>
#include <algorithm>

void processFile(const std::string& filename) {
    // Validate filename: reject shell metacharacters
    auto isUnsafe = [](char c) {
        return c == ';' || c == '&' || c == '|' || c == '$'
            || c == '`' || c == '(' || c == ')';
    };
    if (std::any_of(filename.begin(), filename.end(), isUnsafe)) {
        throw std::invalid_argument("Invalid filename characters");
    }

    // Use fork/exec to avoid shell interpretation
    pid_t pid = fork();
    if (pid == 0) {
        execlp("convert", "convert", filename.c_str(), "output.png", nullptr);
        _exit(127);
    }
    int status;
    waitpid(pid, &status, 0);
    if (WEXITSTATUS(status) != 0) {
        throw std::runtime_error("Conversion failed");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `binary_expression`, `call_expression` (method)
- **Detection approach**: Find `call_expression` nodes where the function is `system` or `popen` and the argument involves a `binary_expression` using `+` on `string` types, or is a variable previously built via string concatenation. Detect `.c_str()` calls on concatenated strings passed to these functions. Also detect `std::string` variables built with `+` or `append()` that flow into `system()`/`popen()`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (call_expression
      function: (field_expression
        field: (field_identifier) @c_str_method)
      arguments: (argument_list))))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_system`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: SQL Injection via String Concatenation in Query Builders

### Description
Building SQL query strings using `std::string` concatenation with user-supplied values and passing them to database library query functions. This bypasses parameterized query support available in libraries like SOCI, libpqxx, and OTL.

### Bad Code (Anti-pattern)
```cpp
#include <soci/soci.h>
#include <string>

std::string getUser(soci::session& sql, const std::string& userId) {
    std::string query = "SELECT * FROM users WHERE id = '" + userId + "'";
    std::string name;
    sql << query, soci::into(name);
    return name;
}

// Using libpqxx
#include <pqxx/pqxx>

pqxx::result searchProducts(pqxx::connection& conn, const std::string& term) {
    pqxx::work txn(conn);
    std::string query = "SELECT * FROM products WHERE name LIKE '%" + term + "%'";
    return txn.exec(query);
}
```

### Good Code (Fix)
```cpp
#include <soci/soci.h>
#include <string>

std::string getUser(soci::session& sql, const std::string& userId) {
    std::string name;
    sql << "SELECT name FROM users WHERE id = :id", soci::use(userId), soci::into(name);
    return name;
}

// Using libpqxx
#include <pqxx/pqxx>

pqxx::result searchProducts(pqxx::connection& conn, const std::string& term) {
    pqxx::work txn(conn);
    std::string searchTerm = "%" + term + "%";
    return txn.exec_params("SELECT * FROM products WHERE name LIKE $1", searchTerm);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `binary_expression`, `string_literal`, `identifier`, `field_expression`
- **Detection approach**: Find `call_expression` nodes where the function is a method like `exec`, `query`, `execute`, or the `<<` operator on a database session, and the argument is a `binary_expression` concatenating a `string_literal` containing SQL keywords with an `identifier` variable. Trace the variable to determine if it originates from user input. Also detect `std::string` variables built with `+` containing SQL fragments that are subsequently passed to query functions.
- **S-expression query sketch**:
```scheme
(declaration
  declarator: (init_declarator
    declarator: (identifier) @var_name
    value: (binary_expression
      left: (string_literal) @sql_fragment
      right: (identifier) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_concat_query`
- **Severity**: error
- **Confidence**: high
