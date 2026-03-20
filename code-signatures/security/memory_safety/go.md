# Memory Safety -- Go

## Overview
Go is a memory-safe language with garbage collection and built-in bounds checking on slice/array access. However, Go does not check for integer overflow in arithmetic operations, and the language allows unchecked slice operations that can lead to out-of-bounds access when indices are derived from user input without validation.

## Why It's a Security Concern
Integer overflow in Go silently wraps (both signed and unsigned types). When overflow occurs in a length or capacity calculation, the resulting value may be much smaller than expected, leading to undersized allocations and subsequent data corruption. Similarly, while Go panics on out-of-bounds slice access, this causes denial of service in production servers unless recovered, and sub-slicing with computed bounds can silently produce incorrect results.

## Applicability
- **Relevance**: medium
- **Languages covered**: .go
- **Frameworks/libraries**: standard library (encoding/binary, io, net/http)

---

## Pattern 1: Integer Overflow in Length Calculations

### Description
Performing arithmetic operations (multiplication, addition) on `int`, `int32`, or `uint32` values derived from untrusted input (e.g., parsed headers, protocol fields) and using the result to allocate slices or buffers. Go does not detect integer overflow at runtime -- it silently wraps, potentially producing a small allocation that is then overwritten with more data than it can hold.

### Bad Code (Anti-pattern)
```go
func readRecords(r io.Reader) ([]Record, error) {
    var count uint32
    binary.Read(r, binary.BigEndian, &count)

    var recordSize uint32
    binary.Read(r, binary.BigEndian, &recordSize)

    totalSize := count * recordSize // overflow wraps silently
    buf := make([]byte, totalSize)  // undersized if overflow occurred
    io.ReadFull(r, buf)
    // parse buf into records...
}
```

### Good Code (Fix)
```go
func readRecords(r io.Reader) ([]Record, error) {
    var count uint32
    binary.Read(r, binary.BigEndian, &count)

    var recordSize uint32
    binary.Read(r, binary.BigEndian, &recordSize)

    totalSize := uint64(count) * uint64(recordSize)
    if totalSize > maxAllowedSize {
        return nil, fmt.Errorf("allocation too large: %d", totalSize)
    }
    buf := make([]byte, totalSize)
    io.ReadFull(r, buf)
    // parse buf into records...
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `short_var_declaration`, `call_expression`
- **Detection approach**: Find `binary_expression` with operator `*` or `+` where operands are variables of integer types (especially `uint32`, `int32`, `int`) and the result is passed to `make()` as a length argument. Flag if neither operand is widened to a larger type (`uint64`) and no bounds check precedes the allocation.
- **S-expression query sketch**:
```scheme
(short_var_declaration
  left: (expression_list
    (identifier) @size_var)
  right: (expression_list
    (binary_expression
      left: (identifier) @lhs
      operator: "*"
      right: (identifier) @rhs)))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `integer_overflow`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Slice Bounds Not Validated on User Input

### Description
Using user-controlled values (from HTTP parameters, protocol fields, file headers) as slice indices or sub-slice bounds without validating that they fall within the slice's actual length. While Go will panic on out-of-bounds access (preventing memory corruption), the panic causes denial of service. Sub-slicing with `data[start:end]` where both bounds are unchecked can also produce silently incorrect results when `start > end` is not validated.

### Bad Code (Anti-pattern)
```go
func extractField(data []byte, offset, length int) []byte {
    return data[offset : offset+length] // panics if out of bounds
}

func handlePacket(data []byte) {
    headerLen := int(data[0])
    payload := data[headerLen:] // user-controlled index, no validation
    process(payload)
}
```

### Good Code (Fix)
```go
func extractField(data []byte, offset, length int) ([]byte, error) {
    end := offset + length
    if offset < 0 || length < 0 || end < offset || end > len(data) {
        return nil, fmt.Errorf("invalid bounds: offset=%d length=%d data_len=%d", offset, length, len(data))
    }
    return data[offset:end], nil
}

func handlePacket(data []byte) error {
    if len(data) < 1 {
        return fmt.Errorf("packet too short")
    }
    headerLen := int(data[0])
    if headerLen > len(data) {
        return fmt.Errorf("header length %d exceeds packet size %d", headerLen, len(data))
    }
    payload := data[headerLen:]
    process(payload)
    return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `slice_expression`, `index_expression`, `call_expression`
- **Detection approach**: Find `slice_expression` nodes where the start or end index is a variable (not a literal) and no preceding `if` statement checks that variable against `len()` of the same slice. Also flag `index_expression` where the index is derived from user input (e.g., a field read from a byte slice via type conversion) without a bounds check.
- **S-expression query sketch**:
```scheme
(slice_expression
  operand: (identifier) @slice_var
  start: (identifier) @start_idx)
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `unchecked_slice_bounds`
- **Severity**: error
- **Confidence**: medium
