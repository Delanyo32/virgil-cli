# Memory Safety -- C++

## Overview
C++ inherits all of C's memory safety issues and adds new ones through its object model. Buffer overflows occur via unchecked array access and raw C string functions. Integer overflows in container size calculations produce undersized allocations. Use-after-free manifests uniquely in C++ through dangling references, invalidated iterators, and accessing objects after `std::move`. Modern C++ (smart pointers, RAII) mitigates many issues, but legacy patterns and performance-critical code often bypass these safeguards.

## Why It's a Security Concern
C++ powers browsers, game engines, databases, and system software. Memory safety vulnerabilities in C++ are the dominant source of security bugs in major projects (Chrome, Firefox, Windows). Buffer overflows enable arbitrary code execution. Use-after-free via invalidated iterators or dangling references is especially dangerous because the freed memory may be reallocated for a different purpose, allowing an attacker to control the contents of the dangled reference.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: STL (string, vector, map), Boost, Qt, raw C APIs

---

## Pattern 1: Buffer Overflow via Unchecked Array Access and Raw String Functions

### Description
Using C-style arrays with unchecked index access, raw `new[]` allocations without bounds tracking, or C string functions (`strcpy`, `sprintf`, `strcat`) on `char*` buffers. Also includes using `std::vector::operator[]` (which does not bounds-check) instead of `.at()` when the index is derived from untrusted input.

### Bad Code (Anti-pattern)
```cpp
void process_input(const char* input) {
    char buffer[64];
    strcpy(buffer, input);  // C-style overflow
}

void parse_records(const std::vector<Record>& records, size_t index) {
    auto& record = records[index];  // no bounds check, UB if index >= size
    process(record);
}

class Parser {
    char* buf;
    size_t capacity;
public:
    void append(const char* data, size_t len) {
        memcpy(buf + offset, data, len);  // no check: offset + len > capacity
    }
};
```

### Good Code (Fix)
```cpp
void process_input(const std::string& input) {
    // std::string handles memory automatically
    log_access(input);
}

void parse_records(const std::vector<Record>& records, size_t index) {
    if (index >= records.size()) {
        throw std::out_of_range("index exceeds records size");
    }
    const auto& record = records.at(index);  // bounds-checked
    process(record);
}

class Parser {
    std::vector<char> buf;
public:
    void append(const char* data, size_t len) {
        buf.insert(buf.end(), data, data + len);  // automatic growth
    }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `subscript_expression`
- **Detection approach**: Find `call_expression` calling `strcpy`, `strcat`, `sprintf`, `gets`, `memcpy` (when size arg is not bounded). Also find `subscript_expression` on arrays or vectors where the index is a variable (not a constant) and no preceding bounds check exists. Flag raw `char[]` buffer usage with C string functions.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  (#match? @func "^(strcpy|strcat|sprintf|gets|vsprintf)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `buffer_overflow`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Integer Overflow in Container Size Calculations

### Description
Performing arithmetic on integer types to compute sizes for `std::vector::reserve()`, `std::vector::resize()`, `new[]`, `malloc()`, or `std::allocator::allocate()` without overflow checking. When `width * height * channels` overflows, the resulting allocation is much smaller than expected, and subsequent element writes corrupt the heap.

### Bad Code (Anti-pattern)
```cpp
std::vector<uint8_t> create_image(uint32_t width, uint32_t height, uint32_t channels) {
    size_t size = width * height * channels;  // overflow if product > 2^32
    std::vector<uint8_t> pixels(size);
    return pixels;
}

void read_table(std::istream& in) {
    uint32_t rows, cols;
    in.read(reinterpret_cast<char*>(&rows), 4);
    in.read(reinterpret_cast<char*>(&cols), 4);
    auto* data = new double[rows * cols];  // overflow possible
    in.read(reinterpret_cast<char*>(data), rows * cols * sizeof(double));
    delete[] data;
}
```

### Good Code (Fix)
```cpp
std::vector<uint8_t> create_image(uint32_t width, uint32_t height, uint32_t channels) {
    uint64_t size = static_cast<uint64_t>(width) * height * channels;
    if (size > MAX_IMAGE_SIZE) {
        throw std::length_error("image dimensions too large");
    }
    std::vector<uint8_t> pixels(static_cast<size_t>(size));
    return pixels;
}

void read_table(std::istream& in) {
    uint32_t rows, cols;
    in.read(reinterpret_cast<char*>(&rows), 4);
    in.read(reinterpret_cast<char*>(&cols), 4);
    uint64_t total = static_cast<uint64_t>(rows) * cols;
    if (total > MAX_TABLE_ELEMENTS) {
        throw std::runtime_error("table too large");
    }
    std::vector<double> data(static_cast<size_t>(total));
    in.read(reinterpret_cast<char*>(data.data()), total * sizeof(double));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `call_expression`, `new_expression`, `declaration`
- **Detection approach**: Find `binary_expression` with operator `*` where the result is passed to `vector` constructor, `.reserve()`, `.resize()`, `new[]`, `malloc()`, or `allocator::allocate()`. Flag when operands are `uint32_t`, `int`, or `size_t` variables (not constants) and no `static_cast<uint64_t>` widening or bounds check precedes the operation.
- **S-expression query sketch**:
```scheme
(new_expression
  type: (type_identifier) @type
  declarator: (new_declarator
    (subscript_expression
      index: (binary_expression
        left: (identifier) @lhs
        operator: "*"
        right: (identifier) @rhs))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `integer_overflow`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: Use-After-Free via Dangling References and Invalidated Iterators

### Description
Holding references, pointers, or iterators to container elements after operations that invalidate them. `std::vector::push_back()` may reallocate, invalidating all iterators and references. Returning references to local variables, accessing `std::move`-from objects, and erasing elements while iterating all produce use-after-free or undefined behavior.

### Bad Code (Anti-pattern)
```cpp
std::string& find_or_add(std::vector<std::string>& names, const std::string& name) {
    for (auto& n : names) {
        if (n == name) return n;
    }
    names.push_back(name);
    return names.back();  // if push_back reallocated, any prior references are dangling
}

void process_all(std::vector<int>& items) {
    for (auto it = items.begin(); it != items.end(); ++it) {
        if (*it < 0) {
            items.erase(it);  // invalidates it, ++it is UB
        }
    }
}

const std::string& get_label() {
    std::string label = compute_label();
    return label;  // dangling reference to destroyed local
}
```

### Good Code (Fix)
```cpp
const std::string& find_or_add(std::vector<std::string>& names, const std::string& name) {
    for (const auto& n : names) {
        if (n == name) return n;
    }
    names.push_back(name);
    return names.back();  // OK only if no prior references are held
}

void process_all(std::vector<int>& items) {
    items.erase(
        std::remove_if(items.begin(), items.end(), [](int x) { return x < 0; }),
        items.end());
}

std::string get_label() {
    std::string label = compute_label();
    return label;  // return by value
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `for_statement`, `return_statement`, `reference_declarator`
- **Detection approach**: Find patterns where `push_back()`, `insert()`, `emplace_back()`, or `resize()` is called on a `std::vector` while references or iterators to its elements are live (declared before the modifying call and used after). Also flag `erase()` inside a for-loop iterating the same container. Flag `return` of a reference to a local variable.
- **S-expression query sketch**:
```scheme
(for_range_loop
  right: (identifier) @container
  body: (compound_statement
    (expression_statement
      (call_expression
        function: (field_expression
          argument: (identifier) @obj
          field: (field_identifier) @method)
        (#match? @method "^(push_back|insert|emplace_back|erase)$")))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `memory_mismanagement`
- **Severity**: error
- **Confidence**: medium
