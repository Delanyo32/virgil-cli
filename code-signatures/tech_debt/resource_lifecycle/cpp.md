# Resource Lifecycle -- C++

## Overview
Resources that are acquired but never properly released, or managed inconsistently, cause memory leaks, double-free bugs, and undefined behavior. In C++, the most common manifestations are raw `new`/`delete` usage instead of smart pointers, and Rule of Five violations where a custom destructor is provided without the corresponding copy/move constructors and assignment operators.

## Why It's a Tech Debt Concern
Raw `new`/`delete` bypasses RAII and makes exception-safe resource management nearly impossible -- any exception thrown between `new` and `delete` leaks the allocation. Smart pointers (`std::unique_ptr`, `std::shared_ptr`) solve this by tying resource lifetime to object scope. Rule of Five violations are even more insidious: a class with a custom destructor (managing a raw resource) but default-generated copy operations will double-free the resource when copied, causing crashes, heap corruption, or security vulnerabilities. These bugs often survive testing and surface only in production under specific code paths that trigger copy/move operations.

## Applicability
- **Relevance**: high (resource management is central to C++)
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: Standard Library, Boost, any code using heap allocation

---

## Pattern 1: Raw new/delete Instead of Smart Pointers

### Description
Using `new` to allocate objects on the heap and `delete` (or `delete[]`) to free them, instead of using `std::unique_ptr` or `std::shared_ptr`. Raw ownership makes code exception-unsafe: if an exception is thrown between `new` and `delete`, the memory leaks. It also makes ownership semantics unclear -- who is responsible for deleting the pointer?

### Bad Code (Anti-pattern)
```cpp
// Raw new/delete -- leaked if process() throws
void handleRequest(const Request& req) {
    Response* resp = new Response();
    resp->setStatus(200);
    resp->setBody(process(req));  // If this throws, resp is leaked
    sendResponse(resp);
    delete resp;
}

// Array allocation with raw delete[]
std::string readFile(const std::string& path) {
    std::ifstream file(path);
    file.seekg(0, std::ios::end);
    size_t size = file.tellg();
    file.seekg(0, std::ios::beg);

    char* buffer = new char[size + 1];
    file.read(buffer, size);
    buffer[size] = '\0';
    std::string content(buffer);
    delete[] buffer;  // Leaked if file.read() throws
    return content;
}

// Factory function returning raw pointer -- who deletes it?
Connection* createConnection(const std::string& host, int port) {
    Connection* conn = new Connection(host, port);
    conn->connect();
    return conn;  // Caller must remember to delete -- not enforced
}

// Mixed ownership -- delete in wrong scope
void processItems(const std::vector<Item>& items) {
    std::vector<Result*> results;
    for (const auto& item : items) {
        results.push_back(new Result(item.compute()));
    }
    for (auto* r : results) {
        r->save();
    }
    for (auto* r : results) {
        delete r;  // If save() throws, remaining Results leak
    }
}
```

### Good Code (Fix)
```cpp
// unique_ptr -- automatic cleanup on all paths
void handleRequest(const Request& req) {
    auto resp = std::make_unique<Response>();
    resp->setStatus(200);
    resp->setBody(process(req));  // Exception-safe: resp cleaned up
    sendResponse(*resp);
}

// No manual allocation needed
std::string readFile(const std::string& path) {
    std::ifstream file(path);
    std::ostringstream ss;
    ss << file.rdbuf();
    return ss.str();
}

// Factory returns unique_ptr -- ownership is explicit
std::unique_ptr<Connection> createConnection(const std::string& host, int port) {
    auto conn = std::make_unique<Connection>(host, port);
    conn->connect();
    return conn;
}

// Container of unique_ptrs -- automatic cleanup
void processItems(const std::vector<Item>& items) {
    std::vector<std::unique_ptr<Result>> results;
    for (const auto& item : items) {
        results.push_back(std::make_unique<Result>(item.compute()));
    }
    for (const auto& r : results) {
        r->save();
    }
    // All Results automatically freed when vector goes out of scope
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `new_expression`, `delete_expression`, `declaration`
- **Detection approach**: Find `new_expression` nodes in function bodies. Check if the result is assigned to a raw pointer type (`T*`) rather than a `std::unique_ptr` or `std::shared_ptr`. Also find `delete_expression` nodes as a secondary signal. Flag any `new` expression that is not passed directly to a smart pointer constructor or `make_unique`/`make_shared`.
- **S-expression query sketch**:
  ```scheme
  ;; Raw new expression
  (new_expression
    type: (type_identifier) @allocated_type)

  ;; delete expression
  (delete_expression
    (identifier) @deleted_var)

  ;; Raw pointer declaration
  (declaration
    type: (type_identifier) @type
    declarator: (init_declarator
      declarator: (pointer_declarator
        declarator: (identifier) @var_name)
      value: (new_expression)))
  ```

### Pipeline Mapping
- **Pipeline name**: `raw_memory_management`
- **Pattern name**: `raw_new_delete`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Rule of Five Violation

### Description
A class that defines a custom destructor (indicating it manages a resource) but does not explicitly define or delete the copy constructor, copy assignment operator, move constructor, and move assignment operator. The compiler-generated defaults will perform shallow copies of raw pointers, leading to double-free bugs, use-after-free, and resource leaks when objects are copied or moved.

### Bad Code (Anti-pattern)
```cpp
// Custom destructor but no copy/move operations defined
class Buffer {
    char* data_;
    size_t size_;

public:
    Buffer(size_t size) : data_(new char[size]), size_(size) {}
    ~Buffer() { delete[] data_; }
    // Compiler generates:
    //   Buffer(const Buffer&) -- shallow copies data_ pointer
    //   Buffer& operator=(const Buffer&) -- shallow copies data_ pointer
    //   Buffer(Buffer&&) -- shallow copies data_ pointer
    //   Buffer& operator=(Buffer&&) -- shallow copies data_ pointer
    // All of these cause double-free when both objects are destroyed

    char* get() { return data_; }
    size_t size() const { return size_; }
};

// Another common case -- file handle management
class FileWrapper {
    FILE* file_;
public:
    FileWrapper(const char* path) : file_(fopen(path, "r")) {}
    ~FileWrapper() { if (file_) fclose(file_); }
    // Default copy: two FileWrappers pointing to same FILE*
    // Both destructors call fclose -- double close / UB
};

// Resource handle without copy semantics
class DatabaseConnection {
    void* handle_;
public:
    DatabaseConnection(const std::string& dsn) : handle_(db_connect(dsn.c_str())) {}
    ~DatabaseConnection() { if (handle_) db_disconnect(handle_); }
    // Copying this object causes double-disconnect
};
```

### Good Code (Fix)
```cpp
// Full Rule of Five implementation
class Buffer {
    char* data_;
    size_t size_;

public:
    Buffer(size_t size) : data_(new char[size]), size_(size) {}
    ~Buffer() { delete[] data_; }

    // Copy constructor -- deep copy
    Buffer(const Buffer& other) : data_(new char[other.size_]), size_(other.size_) {
        std::memcpy(data_, other.data_, size_);
    }

    // Copy assignment -- copy-and-swap
    Buffer& operator=(const Buffer& other) {
        if (this != &other) {
            Buffer tmp(other);
            std::swap(data_, tmp.data_);
            std::swap(size_, tmp.size_);
        }
        return *this;
    }

    // Move constructor
    Buffer(Buffer&& other) noexcept : data_(other.data_), size_(other.size_) {
        other.data_ = nullptr;
        other.size_ = 0;
    }

    // Move assignment
    Buffer& operator=(Buffer&& other) noexcept {
        if (this != &other) {
            delete[] data_;
            data_ = other.data_;
            size_ = other.size_;
            other.data_ = nullptr;
            other.size_ = 0;
        }
        return *this;
    }

    char* get() { return data_; }
    size_t size() const { return size_; }
};

// Or -- use unique_ptr and follow Rule of Zero
class Buffer {
    std::unique_ptr<char[]> data_;
    size_t size_;

public:
    Buffer(size_t size) : data_(std::make_unique<char[]>(size)), size_(size) {}
    // No destructor needed -- unique_ptr handles cleanup
    // Move operations auto-generated correctly
    // Copy operations auto-deleted (unique_ptr is move-only)

    char* get() { return data_.get(); }
    size_t size() const { return size_; }
};

// Non-copyable resource wrapper
class DatabaseConnection {
    void* handle_;
public:
    DatabaseConnection(const std::string& dsn) : handle_(db_connect(dsn.c_str())) {}
    ~DatabaseConnection() { if (handle_) db_disconnect(handle_); }

    // Delete copy operations
    DatabaseConnection(const DatabaseConnection&) = delete;
    DatabaseConnection& operator=(const DatabaseConnection&) = delete;

    // Enable move operations
    DatabaseConnection(DatabaseConnection&& other) noexcept : handle_(other.handle_) {
        other.handle_ = nullptr;
    }
    DatabaseConnection& operator=(DatabaseConnection&& other) noexcept {
        if (this != &other) {
            if (handle_) db_disconnect(handle_);
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_specifier`, `destructor_name`, `function_definition`, `declaration`
- **Detection approach**: Find `class_specifier` nodes that contain a destructor definition (a `function_definition` with a `destructor_name`). Then check if the class body also contains declarations or definitions for: (1) copy constructor (constructor taking `const ClassName&`), (2) copy assignment operator (`operator=` taking `const ClassName&`), (3) move constructor (constructor taking `ClassName&&`), (4) move assignment operator (`operator=` taking `ClassName&&`). Flag classes that have a destructor but are missing any of these four special member functions.
- **S-expression query sketch**:
  ```scheme
  ;; Class with destructor
  (class_specifier
    name: (type_identifier) @class_name
    body: (field_declaration_list
      (function_definition
        declarator: (function_declarator
          declarator: (destructor_name) @dtor_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `rule_of_five`
- **Pattern name**: `incomplete_special_members`
- **Severity**: warning
- **Confidence**: high
