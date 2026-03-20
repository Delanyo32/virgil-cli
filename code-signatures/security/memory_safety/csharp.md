# Memory Safety -- C#

## Overview
C# is a memory-safe language under normal operation -- the CLR prevents buffer overflows and handles memory management via garbage collection. However, `unsafe` blocks and `fixed` statements allow raw pointer manipulation, and P/Invoke marshaling can introduce buffer overflows when managed buffers are incorrectly sized for native function calls. These escape hatches reintroduce the full spectrum of memory corruption vulnerabilities.

## Why It's a Security Concern
When C# code uses `unsafe`/`fixed` blocks or calls native libraries via `[DllImport]`, it leaves the safety of the managed runtime. A native function writing beyond a marshaled buffer's bounds corrupts the managed heap. `stackalloc` in unsafe contexts can overflow the stack. These vulnerabilities are especially dangerous because developers and code reviewers often assume C# code is inherently safe.

## Applicability
- **Relevance**: low
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Runtime.InteropServices (P/Invoke), System.Runtime.CompilerServices.Unsafe

---

## Pattern 1: Buffer Overflow in Unsafe/Fixed Blocks or P/Invoke Marshaling

### Description
Using `unsafe` blocks with raw pointer arithmetic, `fixed` statements that pin managed arrays for pointer access, or `[DllImport]` P/Invoke calls where the managed buffer is smaller than what the native function expects to write. This includes passing `StringBuilder` or `byte[]` with insufficient capacity to native functions that write into them.

### Bad Code (Anti-pattern)
```csharp
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
static extern uint GetWindowsDirectory(StringBuilder lpBuffer, uint uSize);

public string GetWinDir()
{
    var sb = new StringBuilder(8);  // too small
    GetWindowsDirectory(sb, 260);   // native function may write up to 260 chars
    return sb.ToString();           // heap corruption if path > 8 chars
}

unsafe void CopyData(byte[] src, byte[] dst)
{
    fixed (byte* pSrc = src, pDst = dst)
    {
        for (int i = 0; i < src.Length; i++)
        {
            pDst[i] = pSrc[i]; // overflow if dst.Length < src.Length
        }
    }
}
```

### Good Code (Fix)
```csharp
[DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
static extern uint GetWindowsDirectory(StringBuilder lpBuffer, uint uSize);

public string GetWinDir()
{
    const int bufSize = 260;
    var sb = new StringBuilder(bufSize);
    GetWindowsDirectory(sb, (uint)bufSize); // buffer matches declared size
    return sb.ToString();
}

void CopyData(byte[] src, byte[] dst)
{
    if (src.Length > dst.Length)
        throw new ArgumentException("Destination buffer too small");
    Buffer.BlockCopy(src, 0, dst, 0, src.Length); // safe managed copy
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `unsafe_statement`, `fixed_statement`, `invocation_expression`, `attribute`
- **Detection approach**: Find `unsafe_statement` or `fixed_statement` nodes containing pointer arithmetic (binary expressions on pointer variables). For P/Invoke, find methods with `[DllImport]` attribute and check callers that pass `StringBuilder` or `byte[]` -- flag if the buffer capacity argument is smaller than the size argument passed to the native function, or if the capacity is hardcoded to a small value.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (object_creation_expression
      type: (identifier) @type
      arguments: (argument_list
        (integer_literal) @capacity))
    (#eq? @type "StringBuilder")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `unsafe_buffer_overflow`
- **Severity**: error
- **Confidence**: medium
