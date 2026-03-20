# Error Handling Anti-patterns -- C++

## Overview
Errors that are silently swallowed or caught too broadly make debugging impossible and hide real failures. In C++, the `catch(...)` catch-all and empty catch blocks are the most common anti-patterns, discarding exception information that is critical for diagnosing production failures.

## Why It's a Tech Debt Concern
`catch(...)` catches everything -- `std::exception` subclasses, system errors, and even non-exception types thrown by C libraries -- but provides no access to the exception object, making it impossible to log, inspect, or propagate meaningful error information. Empty catch blocks silently discard exceptions, allowing the program to continue with corrupted state, incomplete transactions, or uninitialized resources. In C++ codebases where RAII and exception safety guarantees are critical, these patterns break fundamental invariants and introduce subtle bugs that manifest far from their origin.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`

---

## Pattern 1: Catch-all with `catch(...)`

### Description
Using `catch(...)` to catch all exceptions without knowing what was thrown. No exception object is available for inspection, logging, or wrapping. This hides the actual error type and makes it impossible to distinguish between recoverable errors and critical failures like `std::bad_alloc`.

### Bad Code (Anti-pattern)
```cpp
void processRequest(const Request& req) {
    try {
        auto data = parsePayload(req.body());
        auto result = computeResult(data);
        sendResponse(result);
    } catch (...) {
        sendErrorResponse(500, "Internal error");
    }
}

std::optional<Config> loadConfig(const std::string& path) {
    try {
        return Config::fromFile(path);
    } catch (...) {
        return std::nullopt;
    }
}

void initializeSubsystems() {
    try {
        initDatabase();
        initCache();
        initMessageQueue();
    } catch (...) {
        std::cerr << "Initialization failed\n";
    }
}
```

### Good Code (Fix)
```cpp
void processRequest(const Request& req) {
    try {
        auto data = parsePayload(req.body());
        auto result = computeResult(data);
        sendResponse(result);
    } catch (const ParseException& e) {
        logger::error("Invalid payload: {}", e.what());
        sendErrorResponse(400, "Invalid request payload");
    } catch (const ComputeException& e) {
        logger::error("Computation failed: {}", e.what());
        sendErrorResponse(500, "Processing error");
    } catch (const std::exception& e) {
        logger::error("Unexpected error: {}", e.what());
        sendErrorResponse(500, "Internal error");
        throw; // Re-throw unexpected exceptions
    }
}

std::optional<Config> loadConfig(const std::string& path) {
    try {
        return Config::fromFile(path);
    } catch (const FileNotFoundException& e) {
        logger::info("Config file not found: {}", e.what());
        return std::nullopt;
    } catch (const ParseException& e) {
        logger::error("Config parse error in {}: {}", path, e.what());
        throw; // Don't silently return nullopt for corrupted configs
    }
}

void initializeSubsystems() {
    try {
        initDatabase();
        initCache();
        initMessageQueue();
    } catch (const DatabaseException& e) {
        logger::critical("Database init failed: {}", e.what());
        throw;
    } catch (const std::exception& e) {
        logger::critical("Subsystem init failed: {}", e.what());
        throw;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `parameter_list`
- **Detection approach**: Find `catch_clause` nodes where the parameter declaration is `...` (the catch-all specifier). In the C++ tree-sitter grammar, this is represented as a catch clause with a parameter list containing only an ellipsis. Flag all occurrences, as `catch(...)` is almost always an anti-pattern.
- **S-expression query sketch**:
```scheme
(try_statement
  body: (compound_statement)
  (catch_clause
    parameters: (parameter_list
      (parameter_declaration
        type: (type_identifier) @catch_type))
    body: (compound_statement) @catch_body))

;; catch(...) -- catch-all
(catch_clause
  (parameter_list) @params
  body: (compound_statement) @catch_body)
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `catch_all_ellipsis`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Empty Catch Block

### Description
A `catch` block that contains no statements, discarding the caught exception entirely. The exception is acknowledged by the catch but no action is taken -- no logging, no rethrowing, no alternative handling. This is particularly dangerous in C++ where exceptions carry RAII cleanup implications.

### Bad Code (Anti-pattern)
```cpp
void saveDocument(const Document& doc) {
    try {
        auto data = doc.serialize();
        writeToFile(doc.path(), data);
    } catch (const std::exception& e) {
    }
}

void closeConnections(std::vector<Connection>& conns) {
    for (auto& conn : conns) {
        try {
            conn.close();
        } catch (const ConnectionException&) {
            // Ignore close errors
        }
    }
}

std::unique_ptr<Resource> acquireResource(const std::string& name) {
    try {
        return ResourceManager::acquire(name);
    } catch (const ResourceException& e) {
        // TODO: handle this
    }
    return nullptr;
}
```

### Good Code (Fix)
```cpp
void saveDocument(const Document& doc) {
    try {
        auto data = doc.serialize();
        writeToFile(doc.path(), data);
    } catch (const SerializationException& e) {
        logger::error("Failed to serialize document: {}", e.what());
        throw DocumentSaveException("Could not save document", e);
    } catch (const IoException& e) {
        logger::error("Failed to write document to {}: {}", doc.path(), e.what());
        throw DocumentSaveException("Could not write document to disk", e);
    }
}

void closeConnections(std::vector<Connection>& conns) {
    std::vector<std::string> failures;
    for (auto& conn : conns) {
        try {
            conn.close();
        } catch (const ConnectionException& e) {
            logger::warn("Failed to close connection {}: {}", conn.id(), e.what());
            failures.push_back(conn.id());
        }
    }
    if (!failures.empty()) {
        logger::error("{} connections failed to close cleanly", failures.size());
    }
}

std::unique_ptr<Resource> acquireResource(const std::string& name) {
    try {
        return ResourceManager::acquire(name);
    } catch (const ResourceException& e) {
        logger::error("Failed to acquire resource {}: {}", name, e.what());
        throw;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `compound_statement`
- **Detection approach**: Find `catch_clause` nodes whose body `compound_statement` has zero child statements. Also flag catch blocks containing only comments (all children are `comment` nodes). The presence of a typed exception parameter (e.g., `const std::exception& e`) with an empty body is a strong signal of intentional swallowing.
- **S-expression query sketch**:
```scheme
(catch_clause
  parameters: (parameter_list
    (parameter_declaration
      type: (reference_declarator) @exception_type
      declarator: (identifier) @exception_var))
  body: (compound_statement) @catch_body)

;; Also matches unnamed parameter
(catch_clause
  parameters: (parameter_list
    (parameter_declaration
      type: (reference_declarator) @exception_type))
  body: (compound_statement) @catch_body)
```

### Pipeline Mapping
- **Pipeline name**: `error_handling_antipatterns`
- **Pattern name**: `empty_catch_block`
- **Severity**: warning
- **Confidence**: high
