# Race Conditions -- PHP

## Overview
PHP's traditional request-per-process model limits in-process concurrency, but TOCTOU (time-of-check-to-time-of-use) race conditions in file operations remain a concern. When PHP code checks a file's existence or properties with `file_exists()`, `is_writable()`, or similar functions and then operates on that file, the file system state can change between the two operations. This is particularly relevant in shared hosting environments and applications that use the filesystem for caching, session storage, or configuration management.

## Why It's a Security Concern
TOCTOU races in PHP file operations can be exploited through symlink attacks on shared hosting (replacing a checked path with a symlink to another user's files), cache poisoning by modifying cache files between a freshness check and a read, and privilege escalation in applications that check file permissions before writing security-sensitive configuration. While the exploitation window is narrow, automated tools can win the race reliably.

## Applicability
- **Relevance**: low
- **Languages covered**: .php
- **Frameworks/libraries**: PHP filesystem functions (file_exists, is_readable, fopen), Laravel (File facade), Symfony (Filesystem component)

---

## Pattern 1: TOCTOU in File Operations

### Description
Using `file_exists()`, `is_file()`, `is_readable()`, or `is_writable()` to check a file's state, then operating on the file based on the result. Between the check and the operation, the file can be replaced, deleted, or symlinked by a concurrent process. This is especially dangerous in shared hosting where multiple users' PHP processes access overlapping filesystem paths.

### Bad Code (Anti-pattern)
```php
function readConfig(string $path): string {
    if (file_exists($path)) {
        // RACE: file can be replaced with symlink to /etc/shadow between check and read
        return file_get_contents($path);
    }
    return '';
}

function writeCache(string $path, string $data): void {
    if (!file_exists($path)) {
        // RACE: file or symlink can appear between check and write
        file_put_contents($path, $data);
    }
}
```

### Good Code (Fix)
```php
function readConfig(string $path): string {
    // Attempt the operation directly and handle failure
    $content = @file_get_contents($path);
    if ($content === false) {
        return '';
    }
    return $content;
}

function writeCache(string $path, string $data): void {
    // Use LOCK_EX for atomic write, and write-to-temp + rename for atomicity
    $tmpPath = $path . '.tmp.' . getmypid();
    if (file_put_contents($tmpPath, $data, LOCK_EX) !== false) {
        rename($tmpPath, $path);  // rename is atomic on POSIX
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `if_statement`, `name`, `argument`
- **Detection approach**: Find `if_statement` nodes whose condition contains a `function_call_expression` calling `file_exists`, `is_file`, `is_readable`, `is_writable`, or `is_dir`, where the body contains a `function_call_expression` calling `file_get_contents`, `file_put_contents`, `fopen`, `unlink`, `rename`, or `copy` with the same path variable. The two-step pattern indicates a TOCTOU race.
- **S-expression query sketch**:
```scheme
(if_statement
  condition: (function_call_expression
    function: (name) @check_func
    (#match? @check_func "^(file_exists|is_file|is_readable|is_writable)$")
    arguments: (arguments
      (argument
        (variable_name) @path_var)))
  body: (compound_statement
    (expression_statement
      (function_call_expression
        function: (name) @action_func
        arguments: (arguments
          (argument
            (variable_name) @action_path))))))
```

### Pipeline Mapping
- **Pipeline name**: `toctou`
- **Pattern name**: `file_exists_then_read`
- **Severity**: warning
- **Confidence**: medium
