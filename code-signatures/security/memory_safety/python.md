# Memory Safety -- Python

## Overview
Python is a memory-safe language with garbage collection, making traditional memory corruption vulnerabilities extremely rare in pure Python code. However, when Python interfaces with C extensions via `ctypes` or `cffi`, it can introduce buffer overflow vulnerabilities by passing incorrect buffer sizes or types to native functions, bypassing Python's safety guarantees.

## Why It's a Security Concern
Python applications frequently use `ctypes` to call system libraries or custom C extensions. If the buffer size passed to a C function is smaller than what the function expects to write, the native code will overflow the buffer, corrupting adjacent memory. This can lead to crashes, data corruption, or arbitrary code execution -- all within a language that developers assume is memory-safe.

## Applicability
- **Relevance**: low
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: ctypes, cffi, struct

---

## Pattern 1: Buffer Overflow via ctypes

### Description
Using `ctypes.create_string_buffer()` or `ctypes.c_buffer()` with a hardcoded or incorrectly calculated size, then passing the buffer to a C function that writes more data than the buffer can hold. Also includes passing Python strings directly to C functions expecting fixed-size buffers.

### Bad Code (Anti-pattern)
```python
import ctypes

libc = ctypes.CDLL("libc.so.6")

def get_hostname():
    buf = ctypes.create_string_buffer(8)  # too small for most hostnames
    libc.gethostname(buf, 256)  # tells C it can write 256 bytes into 8-byte buffer
    return buf.value.decode()
```

### Good Code (Fix)
```python
import ctypes

libc = ctypes.CDLL("libc.so.6")

def get_hostname():
    buf_size = 256
    buf = ctypes.create_string_buffer(buf_size)
    libc.gethostname(buf, buf_size)  # buffer size matches the length argument
    return buf.value.decode()
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`, `argument_list`
- **Detection approach**: Find calls to `ctypes.create_string_buffer()` or `ctypes.c_buffer()` that create a buffer, then find subsequent calls to C functions (via a loaded CDLL/WinDLL) that pass a different size argument than the buffer's actual size. Flag cases where the size argument to the C function exceeds the buffer creation size.
- **S-expression query sketch**:
```scheme
(assignment
  left: (identifier) @buf_var
  right: (call
    function: (attribute
      object: (identifier) @mod
      attribute: (identifier) @func)
    arguments: (argument_list
      (integer) @buf_size))
  (#eq? @mod "ctypes")
  (#eq? @func "create_string_buffer"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `ctypes_buffer_overflow`
- **Severity**: error
- **Confidence**: medium
