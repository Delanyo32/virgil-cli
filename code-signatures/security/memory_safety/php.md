# Memory Safety -- PHP

## Overview
PHP is a memory-safe language -- the Zend engine manages memory allocation and prevents direct pointer manipulation. However, the `unpack()` function can cause buffer overflow-like behavior when the format string specifies reading more data than the input string contains, or when incorrect format specifiers are used, leading to unexpected memory reads from the PHP process.

## Why It's a Security Concern
`unpack()` reads binary data according to a format string. If the format string specifies a larger read than the data contains, PHP may read beyond the string's bounds (depending on the PHP version and format specifier), potentially exposing adjacent memory contents. Even in newer PHP versions that return `false` on overflow, incorrect format strings lead to data corruption and logic errors when processing binary protocols, file formats, or network packets.

## Applicability
- **Relevance**: low
- **Languages covered**: .php
- **Frameworks/libraries**: PHP core (pack/unpack), ext-sockets, ext-ffi

---

## Pattern 1: Buffer Overflow via unpack() with Incorrect Format Strings

### Description
Using `unpack()` with a format string that specifies reading more bytes than the input data contains, or using user-controlled data to determine the format string or repeat count. This can cause out-of-bounds reads or produce garbled data that cascades into further vulnerabilities.

### Bad Code (Anti-pattern)
```php
function parseHeader($data) {
    // Format reads 4 unsigned longs (16 bytes) but $data might be shorter
    $header = unpack('N4field', $data);
    $count = $header['field1'];

    // User-controlled repeat count in format string
    $records = unpack("N{$count}val", substr($data, 16));
    return $records;
}

function readPacket($socket) {
    $raw = socket_read($socket, 4);
    $header = unpack('Nlen', $raw);
    // No validation of $header['len'] before using it
    $body = socket_read($socket, $header['len']); // could be huge
    $fields = unpack('N*', $body);
    return $fields;
}
```

### Good Code (Fix)
```php
function parseHeader($data) {
    if (strlen($data) < 16) {
        throw new \InvalidArgumentException('Header too short');
    }
    $header = unpack('N4field', $data);
    $count = $header['field1'];

    $expectedSize = $count * 4;
    $remaining = substr($data, 16);
    if (strlen($remaining) < $expectedSize || $count > 10000) {
        throw new \InvalidArgumentException('Invalid record count');
    }
    $records = unpack("N{$count}val", $remaining);
    return $records;
}

function readPacket($socket) {
    $raw = socket_read($socket, 4);
    if ($raw === false || strlen($raw) < 4) {
        throw new \RuntimeException('Failed to read header');
    }
    $header = unpack('Nlen', $raw);
    if ($header['len'] > MAX_PACKET_SIZE) {
        throw new \RuntimeException('Packet too large');
    }
    $body = socket_read($socket, $header['len']);
    $fields = unpack('N*', $body);
    return $fields;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `argument`, `encapsed_string`, `variable_name`
- **Detection approach**: Find `function_call_expression` calling `unpack` where the format string contains a variable (interpolated or concatenated) as a repeat count, or where no length check on the data argument precedes the call. Flag if the data variable is not validated with `strlen()` before the `unpack()` call.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func
  arguments: (arguments
    (argument
      (encapsed_string
        (variable_name) @repeat_var)))
  (#eq? @func "unpack"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `unpack_buffer_overflow`
- **Severity**: error
- **Confidence**: medium
