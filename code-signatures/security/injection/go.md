# Injection -- Go

## Overview
Injection vulnerabilities in Go arise when user-controlled input is interpolated into SQL queries or shell commands using string formatting functions like `fmt.Sprintf`. Although Go's `database/sql` package provides excellent parameterized query support and `exec.Command` avoids shell invocation by default, developers often bypass these safeguards by building strings manually.

## Why It's a Security Concern
SQL injection can lead to unauthorized data access, modification, or deletion. Command injection allows attackers to execute arbitrary system commands with the application's privileges. Go services frequently run as backend APIs handling external input, making these vulnerabilities especially impactful in production environments.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: database/sql, GORM, sqlx, os/exec, pgx

---

## Pattern 1: SQL Injection via fmt.Sprintf in db.Query

### Description
Using `fmt.Sprintf` or string concatenation to build SQL query strings with user-supplied values, then passing the result to `db.Query()`, `db.Exec()`, or `db.QueryRow()`. This bypasses the parameterized query mechanism (`$1`, `?` placeholders) provided by `database/sql`.

### Bad Code (Anti-pattern)
```go
func GetUser(db *sql.DB, userID string) (*User, error) {
    query := fmt.Sprintf("SELECT * FROM users WHERE id = '%s'", userID)
    row := db.QueryRow(query)
    var user User
    err := row.Scan(&user.ID, &user.Name, &user.Email)
    return &user, err
}
```

### Good Code (Fix)
```go
func GetUser(db *sql.DB, userID string) (*User, error) {
    row := db.QueryRow("SELECT * FROM users WHERE id = $1", userID)
    var user User
    err := row.Scan(&user.ID, &user.Name, &user.Email)
    return &user, err
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `argument_list`, `interpreted_string_literal`
- **Detection approach**: Find `call_expression` nodes where the function is a `selector_expression` ending in `Query`, `QueryRow`, `Exec`, or `QueryContext`, and the first argument is a `call_expression` to `fmt.Sprintf` (or a variable assigned from `fmt.Sprintf`). Confirm SQL keywords in the format string argument.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    field: (field_identifier) @method)
  arguments: (argument_list
    (call_expression
      function: (selector_expression
        operand: (identifier) @pkg
        field: (field_identifier) @fmt_func)
      arguments: (argument_list
        (interpreted_string_literal) @sql_format))))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_sprintf_query`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via exec.Command with User Input

### Description
Passing user-controlled input to `exec.Command` when invoking a shell (`sh`, `bash`, `cmd`) with the `-c` flag, or using user input as the program name itself. While `exec.Command` does not use a shell by default, wrapping user input in `sh -c` reintroduces all shell injection risks.

### Bad Code (Anti-pattern)
```go
func RunTool(userInput string) (string, error) {
    cmd := exec.Command("sh", "-c", fmt.Sprintf("tool --target %s", userInput))
    output, err := cmd.CombinedOutput()
    return string(output), err
}

func RunProgram(programName string) (string, error) {
    cmd := exec.Command(programName)
    output, err := cmd.Output()
    return string(output), err
}
```

### Good Code (Fix)
```go
func RunTool(userInput string) (string, error) {
    // Pass arguments directly without shell invocation
    cmd := exec.Command("tool", "--target", userInput)
    output, err := cmd.CombinedOutput()
    return string(output), err
}

func RunProgram(programName string) (string, error) {
    // Validate against an allowlist
    allowed := map[string]bool{"tool1": true, "tool2": true}
    if !allowed[programName] {
        return "", fmt.Errorf("program %q not allowed", programName)
    }
    cmd := exec.Command(programName)
    output, err := cmd.Output()
    return string(output), err
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `argument_list`, `interpreted_string_literal`
- **Detection approach**: Find `call_expression` nodes calling `exec.Command` where the first argument is a string literal `"sh"`, `"bash"`, or `"cmd"`, the second argument is `"-c"`, and the third argument contains `fmt.Sprintf` or a variable with user input. Also detect cases where the first argument to `exec.Command` is a variable (not a string literal), indicating user-controlled program name.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @func)
  arguments: (argument_list
    (interpreted_string_literal) @shell
    (interpreted_string_literal) @flag
    (call_expression
      function: (selector_expression
        field: (field_identifier) @fmt_func))))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_exec`
- **Severity**: error
- **Confidence**: high
