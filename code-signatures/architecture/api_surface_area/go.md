# API Surface Area -- Go

## Overview
API surface area in Go is governed by a simple naming convention: identifiers starting with an uppercase letter are exported and visible to other packages, while lowercase identifiers are package-private. This implicit visibility mechanism makes it easy to accidentally export symbols. Tracking the ratio of exported to total symbols per file reveals packages that expose too much, undermining Go's emphasis on small, focused interfaces.

## Why It's an Architecture Concern
Go's package system is the primary encapsulation boundary. Every exported function, type, or variable becomes part of the package's public contract that other packages can import and depend on. A wide exported surface creates coupling between packages, makes refactoring difficult without breaking importers, and violates the Go proverb of keeping interfaces small. Once an identifier is exported and consumed, renaming or removing it is a breaking change. Overly broad APIs also make packages harder to understand and test. Disciplining exports to the minimum necessary set keeps packages focused and maintainable.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```go
package storage

func OpenDatabase(dsn string) (*DB, error)     { return nil, nil }
func CloseDatabase(db *DB) error                { return nil }
func Ping(db *DB) error                         { return nil }
func RunMigrations(db *DB) error                { return nil }
func BuildQuery(table string) string            { return "" }
func ExecuteQuery(db *DB, q string) error       { return nil }
func ParseRow(row *Row) (Record, error)         { return Record{}, nil }
func SerializeRecord(r Record) ([]byte, error)  { return nil, nil }
func ValidateSchema(s Schema) error             { return nil }
func CompactTable(db *DB, table string) error   { return nil }
func ReindexTable(db *DB, table string) error   { return nil }
func BackupDatabase(db *DB, path string) error  { return nil }
```

### Good Code (Fix)
```go
package storage

// Exported — the public contract
func Open(dsn string) (*DB, error)              { return nil, nil }
func (db *DB) Close() error                     { return nil }
func (db *DB) Exec(query string) error          { return nil }
func (db *DB) Backup(path string) error         { return nil }

// Unexported — internal helpers
func ping(db *DB) error                         { return nil }
func runMigrations(db *DB) error                { return nil }
func buildQuery(table string) string            { return "" }
func parseRow(row *Row) (Record, error)         { return Record{}, nil }
func serializeRecord(r Record) ([]byte, error)  { return nil, nil }
func validateSchema(s Schema) error             { return nil }
func compactTable(db *DB, table string) error   { return nil }
func reindexTable(db *DB, table string) error   { return nil }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`, `type_declaration`
- **Detection approach**: Count all top-level function/method/type declarations. A symbol is exported if its name identifier starts with an uppercase letter. Flag files where total >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match all function declarations
(function_declaration
  name: (identifier) @func.name)

;; Match all method declarations
(method_declaration
  name: (field_identifier) @method.name)

;; Match all type declarations
(type_declaration
  (type_spec
    name: (type_identifier) @type.name))

;; Post-process: check if name starts with uppercase
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```go
package cache

type Store struct {
    Entries    map[string][]byte
    MaxSize    int
    TTL        time.Duration
    EvictList  *list.List
    Mutex      sync.Mutex
    HitCount   int64
    MissCount  int64
}

func NewStore(maxSize int) *Store {
    return &Store{Entries: make(map[string][]byte), MaxSize: maxSize}
}
```

### Good Code (Fix)
```go
package cache

type Store struct {
    entries    map[string][]byte
    maxSize    int
    ttl        time.Duration
    evictList  *list.List
    mu         sync.Mutex
    hitCount   int64
    missCount  int64
}

func NewStore(maxSize int, ttl time.Duration) *Store {
    return &Store{entries: make(map[string][]byte), maxSize: maxSize, ttl: ttl}
}

func (s *Store) Get(key string) ([]byte, bool) { return nil, false }
func (s *Store) Set(key string, val []byte)    {}
func (s *Store) Stats() (hits, misses int64)   { return s.hitCount, s.missCount }
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration_list` inside `struct_type`, `field_declaration`
- **Detection approach**: Find exported struct types (uppercase name) and inspect their field declarations. A field is exported if its name starts with uppercase. Flag structs where most fields are exported, especially when field types are concrete implementation types (e.g., `sync.Mutex`, `*list.List`).
- **S-expression query sketch**:
```scheme
;; Match struct fields in exported types
(type_declaration
  (type_spec
    name: (type_identifier) @struct.name
    type: (struct_type
      (field_declaration_list
        (field_declaration
          name: (field_identifier) @field.name
          type: (_) @field.type)))))

;; Post-process: check @struct.name starts uppercase, @field.name starts uppercase
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
