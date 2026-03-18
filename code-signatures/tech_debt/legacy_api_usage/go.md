# Legacy API Usage -- Go

## Overview
Legacy API usage in Go refers to relying on deprecated standard library packages or misusing initialization patterns when cleaner, more explicit alternatives exist. Common examples include complex logic in `init()` functions and using the deprecated `ioutil` package instead of its modern replacements in `io` and `os`.

## Why It's a Tech Debt Concern
The `ioutil` package was officially deprecated in Go 1.16, and its functions were moved to `io` and `os`. Continued use signals unmaintained code and will eventually trigger deprecation warnings in IDEs and linters. Overloading `init()` with complex logic makes programs hard to test, introduces hidden execution order dependencies between packages, and makes startup failures difficult to diagnose because `init()` runs before `main()` with no way to pass errors back.

## Applicability
- **Relevance**: high (both patterns are widespread in Go codebases started before Go 1.16)
- **Languages covered**: `.go`
- **Frameworks/libraries**: N/A (standard library patterns)

---

## Pattern 1: init() Function Abuse

### Description
Placing complex logic -- database connections, HTTP calls, file I/O, goroutine launches, or heavy computation -- inside `init()` functions. Go's `init()` runs automatically at package import time with no way to handle errors, pass configuration, or control execution order across packages. It should be reserved for trivial setup like registering drivers or setting default values.

### Bad Code (Anti-pattern)
```go
package database

import (
    "database/sql"
    "fmt"
    "log"
    "os"

    _ "github.com/lib/pq"
)

var db *sql.DB
var cache *RedisClient
var migrationsDone bool

func init() {
    var err error
    connStr := fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=disable",
        os.Getenv("DB_HOST"),
        os.Getenv("DB_PORT"),
        os.Getenv("DB_USER"),
        os.Getenv("DB_PASSWORD"),
        os.Getenv("DB_NAME"),
    )

    db, err = sql.Open("postgres", connStr)
    if err != nil {
        log.Fatalf("Failed to connect to database: %v", err)
    }

    if err = db.Ping(); err != nil {
        log.Fatalf("Database ping failed: %v", err)
    }

    cache, err = NewRedisClient(os.Getenv("REDIS_URL"))
    if err != nil {
        log.Fatalf("Failed to connect to Redis: %v", err)
    }

    if err = runMigrations(db); err != nil {
        log.Fatalf("Migrations failed: %v", err)
    }
    migrationsDone = true

    go startHealthCheckLoop(db)
}
```

### Good Code (Fix)
```go
package database

import (
    "context"
    "database/sql"
    "fmt"

    _ "github.com/lib/pq"
)

type Config struct {
    Host     string
    Port     string
    User     string
    Password string
    DBName   string
    RedisURL string
}

type Store struct {
    DB    *sql.DB
    Cache *RedisClient
}

func NewStore(ctx context.Context, cfg Config) (*Store, error) {
    connStr := fmt.Sprintf("host=%s port=%s user=%s password=%s dbname=%s sslmode=disable",
        cfg.Host, cfg.Port, cfg.User, cfg.Password, cfg.DBName,
    )

    db, err := sql.Open("postgres", connStr)
    if err != nil {
        return nil, fmt.Errorf("opening database: %w", err)
    }

    if err = db.PingContext(ctx); err != nil {
        return nil, fmt.Errorf("pinging database: %w", err)
    }

    cache, err := NewRedisClient(cfg.RedisURL)
    if err != nil {
        return nil, fmt.Errorf("connecting to redis: %w", err)
    }

    if err = runMigrations(db); err != nil {
        return nil, fmt.Errorf("running migrations: %w", err)
    }

    return &Store{DB: db, Cache: cache}, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration` with name `init` and empty parameter list
- **Detection approach**: Find `function_declaration` nodes with name `init` and no parameters. Count the statements in the function body. Flag when the body contains more than 3 statements, or contains `call_expression` nodes targeting I/O, networking, or concurrency functions (`sql.Open`, `http.Get`, `os.Open`, `go` statements). Trivial `init()` bodies (1-2 simple assignments) should not be flagged.
- **S-expression query sketch**:
```scheme
(function_declaration
  name: (identifier) @func_name
  parameters: (parameter_list)
  body: (block) @init_body
  (#eq? @func_name "init"))
```

### Pipeline Mapping
- **Pipeline name**: `init_abuse`
- **Pattern name**: `complex_init_function`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deprecated ioutil Package

### Description
Using functions from the `io/ioutil` package, which was deprecated in Go 1.16. All `ioutil` functions have direct replacements: `ioutil.ReadAll` -> `io.ReadAll`, `ioutil.ReadFile` -> `os.ReadFile`, `ioutil.WriteFile` -> `os.WriteFile`, `ioutil.TempDir` -> `os.MkdirTemp`, `ioutil.TempFile` -> `os.CreateTemp`, `ioutil.ReadDir` -> `os.ReadDir`, `ioutil.NopCloser` -> `io.NopCloser`, `ioutil.Discard` -> `io.Discard`.

### Bad Code (Anti-pattern)
```go
package main

import (
    "io/ioutil"
    "log"
    "net/http"
)

func readConfig(path string) ([]byte, error) {
    data, err := ioutil.ReadFile(path)
    if err != nil {
        return nil, err
    }
    return data, nil
}

func writeOutput(path string, data []byte) error {
    return ioutil.WriteFile(path, data, 0644)
}

func fetchURL(url string) (string, error) {
    resp, err := http.Get(url)
    if err != nil {
        return "", err
    }
    defer resp.Body.Close()

    body, err := ioutil.ReadAll(resp.Body)
    if err != nil {
        return "", err
    }
    return string(body), nil
}

func listFiles(dir string) {
    files, err := ioutil.ReadDir(dir)
    if err != nil {
        log.Fatal(err)
    }
    for _, f := range files {
        log.Println(f.Name())
    }
}

func createTempFile() (*os.File, error) {
    return ioutil.TempFile("", "prefix-")
}
```

### Good Code (Fix)
```go
package main

import (
    "io"
    "log"
    "net/http"
    "os"
)

func readConfig(path string) ([]byte, error) {
    data, err := os.ReadFile(path)
    if err != nil {
        return nil, err
    }
    return data, nil
}

func writeOutput(path string, data []byte) error {
    return os.WriteFile(path, data, 0644)
}

func fetchURL(url string) (string, error) {
    resp, err := http.Get(url)
    if err != nil {
        return "", err
    }
    defer resp.Body.Close()

    body, err := io.ReadAll(resp.Body)
    if err != nil {
        return "", err
    }
    return string(body), nil
}

func listFiles(dir string) {
    files, err := os.ReadDir(dir)
    if err != nil {
        log.Fatal(err)
    }
    for _, f := range files {
        log.Println(f.Name())
    }
}

func createTempFile() (*os.File, error) {
    return os.CreateTemp("", "prefix-")
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_spec` with `"io/ioutil"` path, `selector_expression` with `ioutil` identifier
- **Detection approach**: Find `import_spec` nodes containing the string `"io/ioutil"`. Additionally, find `selector_expression` nodes where the operand is the identifier `ioutil` and the field is any of `ReadAll`, `ReadFile`, `WriteFile`, `TempDir`, `TempFile`, `ReadDir`, `NopCloser`, or `Discard`. Every occurrence is a candidate for replacement.
- **S-expression query sketch**:
```scheme
(import_spec
  path: (interpreted_string_literal) @import_path
  (#eq? @import_path "\"io/ioutil\""))

(selector_expression
  operand: (identifier) @pkg
  field: (field_identifier) @func
  (#eq? @pkg "ioutil"))
```

### Pipeline Mapping
- **Pipeline name**: `deprecated_ioutil`
- **Pattern name**: `ioutil_function_usage`
- **Severity**: warning
- **Confidence**: high
