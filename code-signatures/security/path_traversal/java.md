# Path Traversal -- Java

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks (database credentials, API keys), or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.io.File, java.nio.file.Path, Spring MVC, Jakarta Servlet, Apache Commons IO

---

## Pattern 1: User Input in File Path

### Description
Constructing a `File` or `Path` using `new File(base, userInput)` or `Paths.get(base, userInput)` without resolving the canonical path via `.getCanonicalPath()` and verifying it starts with the intended base directory.

### Bad Code (Anti-pattern)
```java
@GetMapping("/files/{name}")
public ResponseEntity<byte[]> getFile(@PathVariable String name) throws IOException {
    File file = new File("/var/uploads", name);
    byte[] content = Files.readAllBytes(file.toPath());
    return ResponseEntity.ok(content);
}
```

### Good Code (Fix)
```java
@GetMapping("/files/{name}")
public ResponseEntity<byte[]> getFile(@PathVariable String name) throws IOException {
    File baseDir = new File("/var/uploads").getCanonicalFile();
    File file = new File(baseDir, name).getCanonicalFile();
    if (!file.getPath().startsWith(baseDir.getPath() + File.separator)) {
        return ResponseEntity.status(HttpStatus.FORBIDDEN).build();
    }
    byte[] content = Files.readAllBytes(file.toPath());
    return ResponseEntity.ok(content);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `method_invocation`, `identifier`, `string_literal`
- **Detection approach**: Find `object_creation_expression` nodes creating `new File(base, userInput)` or `method_invocation` nodes calling `Paths.get(base, userInput)` where one argument originates from user input (e.g., `@PathVariable`, `@RequestParam`, `request.getParameter()`). Flag when the result is used for I/O without a preceding `.getCanonicalPath()` + `.startsWith()` check.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (type_identifier) @type_name
  arguments: (argument_list
    (_)
    (identifier) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `user_input_in_file_path`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Directory Traversal via ../

### Description
Accepting file paths that contain `../` or `..\\` sequences without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```java
@GetMapping("/download")
public ResponseEntity<byte[]> download(@RequestParam String file) throws IOException {
    // No check for ".." — attacker sends ?file=../../../etc/passwd
    Path path = Paths.get("./public/" + file);
    byte[] content = Files.readAllBytes(path);
    return ResponseEntity.ok(content);
}
```

### Good Code (Fix)
```java
@GetMapping("/download")
public ResponseEntity<byte[]> download(@RequestParam String file) throws IOException {
    if (file.contains("..")) {
        return ResponseEntity.badRequest().build();
    }
    File baseDir = new File("./public").getCanonicalFile();
    File resolved = new File(baseDir, file).getCanonicalFile();
    if (!resolved.getPath().startsWith(baseDir.getPath() + File.separator)) {
        return ResponseEntity.status(HttpStatus.FORBIDDEN).build();
    }
    byte[] content = Files.readAllBytes(resolved.toPath());
    return ResponseEntity.ok(content);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `binary_expression`, `string_literal`, `identifier`
- **Detection approach**: Find `method_invocation` nodes calling `Paths.get()`, `Files.readAllBytes()`, or `new FileInputStream()` where the path argument is a string concatenation that includes a variable from user input. Flag when there is no preceding check for `".."` via `.contains("..")` or `.getCanonicalPath()` + `.startsWith()` validation.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @obj
  name: (identifier) @method
  arguments: (argument_list
    (binary_expression
      left: (string_literal) @path_prefix
      right: (identifier) @user_var)))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
