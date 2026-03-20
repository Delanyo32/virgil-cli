# Type Safety Gaps -- C

## Overview
C's type system allows `void*` pointers that erase type information and permits implicit conversions between signed and unsigned integers. Both patterns bypass the compiler's ability to catch type mismatches and can lead to undefined behavior, security vulnerabilities, and data corruption.

## Why It's a Tech Debt Concern
Casting to and from `void*` eliminates all type information at the pointer level, meaning the compiler cannot verify that the pointed-to data matches the expected type. Incorrect casts cause memory corruption, buffer overflows, and crashes that are extremely difficult to debug. Signed/unsigned mismatches in comparisons can cause logic errors where negative values become very large positive values (or vice versa), leading to buffer overflows, infinite loops, and incorrect boundary checks -- a class of vulnerability frequently exploited in security attacks.

## Applicability
- **Relevance**: high (void pointers and mixed signedness are fundamental to C programming)
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: All C codebases; Linux kernel, embedded systems, system libraries

---

## Pattern 1: `void*` Pointer Abuse

### Description
Using `void*` as a parameter type, return type, or intermediate variable to pass around data without type information, then casting back to a concrete type at the use site. While `void*` is sometimes necessary for generic data structures in C, overuse eliminates compile-time type safety and introduces risks of casting to the wrong type.

### Bad Code (Anti-pattern)
```c
typedef struct {
    void *data;
    int type;
} Container;

void process(void *data, int type) {
    if (type == TYPE_INT) {
        int *val = (int *)data;
        printf("%d\n", *val);
    } else if (type == TYPE_STRING) {
        char *str = (char *)data;
        printf("%s\n", str);
    } else if (type == TYPE_FLOAT) {
        float *f = (float *)data;  // Wrong if data is actually double*
        printf("%f\n", *f);
    }
}

void *create_buffer(int size) {
    void *buf = malloc(size);
    return buf;
}

void store_value(Container *c, void *value) {
    c->data = value;  // No type verification
}
```

### Good Code (Fix)
```c
typedef struct {
    int value;
} IntContainer;

typedef struct {
    char value[256];
} StringContainer;

typedef struct {
    double value;
} FloatContainer;

void process_int(const IntContainer *data) {
    printf("%d\n", data->value);
}

void process_string(const StringContainer *data) {
    printf("%s\n", data->value);
}

void process_float(const FloatContainer *data) {
    printf("%f\n", data->value);
}

/* When void* is truly needed, use typed wrapper with size tracking */
typedef struct {
    void *data;
    size_t elem_size;
    size_t count;
} TypedBuffer;

TypedBuffer create_int_buffer(size_t count) {
    TypedBuffer buf = {
        .data = calloc(count, sizeof(int)),
        .elem_size = sizeof(int),
        .count = count,
    };
    return buf;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `parameter_declaration`, `cast_expression`, `type_descriptor`, `primitive_type`
- **Detection approach**: Find `parameter_declaration` and `declaration` nodes where the type includes `void` followed by a pointer declarator (`*`). Also find `cast_expression` nodes that cast to or from `void *`. Flag function parameters typed as `void *`, return types of `void *`, and explicit casts from `void *` to concrete pointer types.
- **S-expression query sketch**:
```scheme
(parameter_declaration
  type: (primitive_type) @type
  declarator: (pointer_declarator) @ptr
  (#eq? @type "void"))

(cast_expression
  type: (type_descriptor
    type: (primitive_type) @cast_type
    declarator: (abstract_pointer_declarator))
  value: (_) @value)
```

### Pipeline Mapping
- **Pipeline name**: `void_pointer_abuse`
- **Pattern name**: `void_star_cast`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Signed/Unsigned Integer Mismatch in Comparisons

### Description
Comparing signed and unsigned integer values without explicit conversion. When a signed integer is compared to an unsigned integer, C implicitly converts the signed value to unsigned, causing negative values to wrap around to very large positive values. This leads to incorrect comparison results, buffer overflows, and security vulnerabilities.

### Bad Code (Anti-pattern)
```c
void process_data(const char *data, int length) {
    size_t buffer_size = get_buffer_size();
    if (length > buffer_size) {  // signed/unsigned: negative length becomes huge
        return;
    }
    memcpy(buffer, data, length);
}

int find_index(int *arr, size_t size, int target) {
    for (int i = 0; i < size; i++) {  // signed i compared to unsigned size
        if (arr[i] == target) {
            return i;
        }
    }
    return -1;
}

void validate_offset(int offset, unsigned int max) {
    if (offset < max) {  // -1 < 4294967295u is false (wraps to huge positive)
        access(offset);
    }
}

int check_bounds(int index, size_t count) {
    return index >= 0 && index < count;  // second comparison is signed/unsigned
}
```

### Good Code (Fix)
```c
void process_data(const char *data, int length) {
    if (length < 0) {
        return;  // Reject negative lengths explicitly
    }
    size_t buffer_size = get_buffer_size();
    if ((size_t)length > buffer_size) {  // Safe: length is known non-negative
        return;
    }
    memcpy(buffer, data, (size_t)length);
}

int find_index(int *arr, size_t size, int target) {
    for (size_t i = 0; i < size; i++) {  // Use matching unsigned type
        if (arr[i] == target) {
            return (int)i;
        }
    }
    return -1;
}

void validate_offset(int offset, unsigned int max) {
    if (offset >= 0 && (unsigned int)offset < max) {
        access(offset);
    }
}

int check_bounds(int index, size_t count) {
    return index >= 0 && (size_t)index < count;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `declaration`, `primitive_type`
- **Detection approach**: Find `binary_expression` nodes with comparison operators (`<`, `>`, `<=`, `>=`, `==`, `!=`). For each operand, trace the identifier to its declaration and check whether one operand is a signed type (`int`, `char`, `short`, `long`) and the other is unsigned (`unsigned int`, `size_t`, `uint32_t`, etc.). Since tree-sitter does not provide type inference, use heuristics: flag comparisons where one operand's declaration uses `size_t` or `unsigned` and the other uses a plain signed type.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (identifier) @left
  operator: ["<" ">" "<=" ">=" "==" "!="] @op
  right: (identifier) @right)
```

### Pipeline Mapping
- **Pipeline name**: `signed_unsigned_mismatch`
- **Pattern name**: `mixed_sign_comparison`
- **Severity**: warning
- **Confidence**: medium
