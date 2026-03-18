# Memory Safety -- Java

## Overview
Java is a memory-safe language -- the JVM prevents buffer overflows, use-after-free, and direct memory corruption. Array bounds are checked at runtime, and the garbage collector handles deallocation. However, integer overflow in array/buffer size calculations is still possible and can lead to logic errors, undersized allocations, and denial of service, even if the JVM prevents actual memory corruption.

## Why It's a Security Concern
While the JVM will throw `ArrayIndexOutOfBoundsException` or `NegativeArraySizeException` rather than allowing memory corruption, integer overflow in size calculations can still be exploited. An attacker-controlled count multiplied by a record size can wrap to a small positive value, causing the application to allocate a tiny array and then throw exceptions or produce corrupted data when processing. In `ByteBuffer` and `Unsafe` usage, the consequences can be more severe.

## Applicability
- **Relevance**: low
- **Languages covered**: .java
- **Frameworks/libraries**: java.nio (ByteBuffer), java.util (Arrays, ArrayList), sun.misc.Unsafe

---

## Pattern 1: Integer Overflow in Array/Buffer Size Calculations

### Description
Multiplying or adding `int` values from untrusted sources to compute array sizes or buffer capacities without overflow checking. Java's `int` is 32-bit signed; `count * recordSize` can wrap to a negative or small positive number, causing `NegativeArraySizeException` or a silently undersized allocation.

### Bad Code (Anti-pattern)
```java
public byte[] readPayload(DataInputStream in) throws IOException {
    int count = in.readInt();       // attacker-controlled
    int recordSize = in.readInt();  // attacker-controlled
    int totalSize = count * recordSize; // integer overflow wraps silently
    byte[] buffer = new byte[totalSize]; // may be negative or tiny
    in.readFully(buffer);
    return buffer;
}
```

### Good Code (Fix)
```java
public byte[] readPayload(DataInputStream in) throws IOException {
    int count = in.readInt();
    int recordSize = in.readInt();
    long totalSize = (long) count * recordSize;
    if (totalSize < 0 || totalSize > MAX_ALLOWED_SIZE) {
        throw new IOException("Invalid payload size: " + totalSize);
    }
    byte[] buffer = new byte[(int) totalSize];
    in.readFully(buffer);
    return buffer;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `local_variable_declaration`, `array_creation_expression`
- **Detection approach**: Find `binary_expression` with operator `*` or `+` on `int` variables where the result is used in `new byte[...]`, `new int[...]`, `ByteBuffer.allocate()`, or `Arrays.copyOf()`. Flag if neither operand is cast to `long` and no `Math.multiplyExact()` or `Math.addExact()` is used.
- **S-expression query sketch**:
```scheme
(local_variable_declaration
  type: (integral_type) @type
  declarator: (variable_declarator
    name: (identifier) @size_var
    value: (binary_expression
      left: (identifier) @lhs
      operator: "*"
      right: (identifier) @rhs))
  (#eq? @type "int"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `integer_overflow`
- **Severity**: warning
- **Confidence**: medium
