# Module Size Distribution -- Go

## Overview
Module size distribution measures how symbol definitions are spread across source files in a Go codebase. Go encourages small, focused files within a package, and balanced file sizes make it easier to understand package responsibilities, keep code reviews manageable, and reduce merge conflicts. Files that are excessively large or contain only a single symbol indicate structural imbalance.

## Why It's an Architecture Concern
Oversized Go files concentrate too many functions, types, and constants into a single file, making it hard to locate specific functionality and increasing the likelihood of merge conflicts. Since Go organizes code at the package level rather than the file level, a bloated file often means the package itself has too many responsibilities. Anemic modules that define only one symbol create unnecessary file fragmentation -- in Go, this is especially wasteful because all files in a package share the same namespace, and splitting a single function into its own file adds navigation overhead without improving encapsulation.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```go
// utils.go -- a dumping ground for the entire package
package server

func HandleLogin(w http.ResponseWriter, r *http.Request) { /* ... */ }
func HandleLogout(w http.ResponseWriter, r *http.Request) { /* ... */ }
func HandleRegister(w http.ResponseWriter, r *http.Request) { /* ... */ }
func ValidateEmail(email string) bool { /* ... */ }
func ValidatePassword(pw string) bool { /* ... */ }
func HashPassword(pw string) (string, error) { /* ... */ }

type User struct { /* ... */ }
type Session struct { /* ... */ }
type Config struct { /* ... */ }

const MaxRetries = 5
const DefaultTimeout = 30

var ErrNotFound = errors.New("not found")
var ErrUnauthorized = errors.New("unauthorized")

// ... 18 more functions, types, and constants
```

### Good Code (Fix)
```go
// auth.go -- focused on authentication
package server

func HandleLogin(w http.ResponseWriter, r *http.Request) { /* ... */ }
func HandleLogout(w http.ResponseWriter, r *http.Request) { /* ... */ }
func HandleRegister(w http.ResponseWriter, r *http.Request) { /* ... */ }
func HashPassword(pw string) (string, error) { /* ... */ }
```

```go
// validation.go -- focused on input validation
package server

func ValidateEmail(email string) bool { /* ... */ }
func ValidatePassword(pw string) bool { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`, `type_declaration`, `var_declaration`, `const_declaration`
- **Detection approach**: Count all top-level declarations (direct children of `source_file`). For `var_declaration` and `const_declaration` blocks, count each spec individually. Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(source_file
  [
    (function_declaration name: (identifier) @name) @def
    (method_declaration name: (field_identifier) @name) @def
    (type_declaration (type_spec name: (type_identifier) @name)) @def
    (var_declaration (var_spec name: (identifier) @name)) @def
    (const_declaration (const_spec name: (identifier) @name)) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```go
// helpers.go -- too many exported symbols
package helpers

func FormatDate(t time.Time) string { /* ... */ }
func ParseDate(s string) (time.Time, error) { /* ... */ }
func FormatCurrency(amount float64) string { /* ... */ }
func TrimWhitespace(s string) string { /* ... */ }
func Capitalize(s string) string { /* ... */ }
func SlugifyString(s string) string { /* ... */ }
func GenerateUUID() string { /* ... */ }
func HashSHA256(data []byte) string { /* ... */ }
func EncodeBase64(data []byte) string { /* ... */ }
func DecodeBase64(s string) ([]byte, error) { /* ... */ }
type StringSet map[string]struct{}
type Pair[T any] struct { First, Second T }
// ... 10 more exported symbols
```

### Good Code (Fix)
```go
// format.go -- focused on formatting
package format

func Date(t time.Time) string { /* ... */ }
func Currency(amount float64) string { /* ... */ }
```

```go
// crypto.go -- focused on hashing and encoding
package crypto

func HashSHA256(data []byte) string { /* ... */ }
func EncodeBase64(data []byte) string { /* ... */ }
func DecodeBase64(s string) ([]byte, error) { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`, `type_declaration`, `var_declaration`, `const_declaration`
- **Detection approach**: Count top-level symbols whose name starts with an uppercase letter (Go's export convention). Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(source_file
  (function_declaration
    name: (identifier) @name
    (#match? @name "^[A-Z]")) @def)

(source_file
  (type_declaration
    (type_spec
      name: (type_identifier) @name
      (#match? @name "^[A-Z]"))) @def)
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```go
// version.go
package app

const Version = "1.2.3"
```

### Good Code (Fix)
```go
// app.go -- merge the trivial constant into a related file
package app

const Version = "1.2.3"

const DefaultPort = 8080

func Name() string {
    return "myapp"
}

func PrintBanner() {
    fmt.Printf("%s v%s\n", Name(), Version)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`, `type_declaration`, `var_declaration`, `const_declaration`
- **Detection approach**: Count top-level symbol definitions (direct children of `source_file`). Flag if count == 1, excluding test files (`_test.go`), `main.go`, and `doc.go`.
- **S-expression query sketch**:
```scheme
(source_file
  [
    (function_declaration name: (identifier) @name) @def
    (method_declaration name: (field_identifier) @name) @def
    (type_declaration (type_spec name: (type_identifier) @name)) @def
    (var_declaration (var_spec name: (identifier) @name)) @def
    (const_declaration (const_spec name: (identifier) @name)) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
