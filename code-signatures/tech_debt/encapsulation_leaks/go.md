# Encapsulation Leaks -- Go

## Overview
Encapsulation leaks in Go occur when functions return concrete struct types instead of interfaces, tightly coupling callers to specific implementations, or when struct fields are exported (capitalized) when they should be unexported with accessor methods to enforce invariants. Both patterns reduce flexibility and make refactoring or testing harder.

## Why It's a Tech Debt Concern
Returning concrete types instead of interfaces prevents callers from substituting implementations for testing or future changes — every consumer is locked to one specific struct. Exported struct fields allow any package to read and modify internal state directly, bypassing validation and invariant enforcement. When a field name or type changes, every external package that references it must be updated, turning internal refactoring into a cross-package breaking change.

## Applicability
- **Relevance**: high (Go's exported/unexported naming convention is the sole encapsulation mechanism)
- **Languages covered**: `.go`
- **Frameworks/libraries**: Standard library patterns (io.Reader/Writer interfaces), Gin/Echo (handler dependencies), gRPC (service implementations)

---

## Pattern 1: Concrete Return Types

### Description
An exported function or method returns a concrete struct pointer (`*MyStruct`) instead of an interface. This forces all callers to depend on the concrete type, making it impossible to swap in alternative implementations, mocks, or decorators without changing every call site.

### Bad Code (Anti-pattern)
```go
type PostgresUserStore struct {
    db *sql.DB
}

func (s *PostgresUserStore) FindByID(id string) (*User, error) { /* ... */ }
func (s *PostgresUserStore) Save(user *User) error { /* ... */ }
func (s *PostgresUserStore) Delete(id string) error { /* ... */ }

// Returns concrete type — callers cannot substitute
func NewUserStore(db *sql.DB) *PostgresUserStore {
    return &PostgresUserStore{db: db}
}

type UserService struct {
    store *PostgresUserStore  // concrete dependency
}

func NewUserService(store *PostgresUserStore) *UserService {
    return &UserService{store: store}
}

func (s *UserService) GetUser(id string) (*User, error) {
    return s.store.FindByID(id)
}

// Tests require a real PostgresUserStore with a database connection
func TestGetUser(t *testing.T) {
    db := setupTestDB(t)  // heavy setup
    store := NewUserStore(db)
    svc := NewUserService(store)
    // ...
}
```

### Good Code (Fix)
```go
type UserStore interface {
    FindByID(id string) (*User, error)
    Save(user *User) error
    Delete(id string) error
}

type PostgresUserStore struct {
    db *sql.DB
}

func (s *PostgresUserStore) FindByID(id string) (*User, error) { /* ... */ }
func (s *PostgresUserStore) Save(user *User) error { /* ... */ }
func (s *PostgresUserStore) Delete(id string) error { /* ... */ }

// Returns interface — callers depend on behavior, not implementation
func NewUserStore(db *sql.DB) UserStore {
    return &PostgresUserStore{db: db}
}

type UserService struct {
    store UserStore  // interface dependency
}

func NewUserService(store UserStore) *UserService {
    return &UserService{store: store}
}

func (s *UserService) GetUser(id string) (*User, error) {
    return s.store.FindByID(id)
}

// Tests use a lightweight mock
type mockUserStore struct {
    users map[string]*User
}

func (m *mockUserStore) FindByID(id string) (*User, error) {
    if u, ok := m.users[id]; ok {
        return u, nil
    }
    return nil, ErrNotFound
}

func TestGetUser(t *testing.T) {
    store := &mockUserStore{users: map[string]*User{"1": {ID: "1", Name: "Alice"}}}
    svc := NewUserService(store)
    // ...
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration` with `result` containing `pointer_type` wrapping a `type_identifier`
- **Detection approach**: Find exported `function_declaration` nodes (name starts with uppercase) where the result type is a `pointer_type` to a concrete `type_identifier` (not an interface). Constructor functions matching `New*` patterns are the primary target. Cross-reference the returned type — if a corresponding interface exists with matching methods, the concrete return is likely unintentional.
- **S-expression query sketch**:
  ```scheme
  (function_declaration
    name: (identifier) @func_name
    result: (pointer_type
      (type_identifier) @return_type))

  (method_declaration
    name: (field_identifier) @method_name
    result: (pointer_type
      (type_identifier) @return_type))
  ```

### Pipeline Mapping
- **Pipeline name**: `concrete_return_type`
- **Pattern name**: `concrete_constructor_return`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Exported Fields That Should Be Unexported

### Description
An exported struct exposes fields with uppercase names that represent internal state, allowing any external package to read and modify them directly. These fields should be unexported (lowercase) with accessor methods that can enforce validation and invariants.

### Bad Code (Anti-pattern)
```go
type Server struct {
    Host         string
    Port         int
    MaxConns     int
    ActiveConns  int        // internal counter — should not be settable
    StartTime    time.Time  // internal state
    Connections  []*Conn    // internal pool
    ShutdownChan chan struct{} // internal signal
    Logger       *log.Logger
}

func NewServer(host string, port int) *Server {
    return &Server{
        Host:         host,
        Port:         port,
        MaxConns:     100,
        ActiveConns:  0,
        StartTime:    time.Now(),
        Connections:  make([]*Conn, 0),
        ShutdownChan: make(chan struct{}),
        Logger:       log.Default(),
    }
}

// External code can break invariants
srv := NewServer("localhost", 8080)
srv.ActiveConns = -5          // invalid state
srv.Connections = nil         // destroy connection pool
close(srv.ShutdownChan)       // trigger shutdown from outside
srv.MaxConns = 0              // no validation
```

### Good Code (Fix)
```go
type Server struct {
    host         string
    port         int
    maxConns     int
    activeConns  int
    startTime    time.Time
    connections  []*Conn
    shutdownChan chan struct{}
    logger       *log.Logger
}

func NewServer(host string, port int, opts ...Option) *Server {
    srv := &Server{
        host:         host,
        port:         port,
        maxConns:     100,
        activeConns:  0,
        startTime:    time.Now(),
        connections:  make([]*Conn, 0),
        shutdownChan: make(chan struct{}),
        logger:       log.Default(),
    }
    for _, opt := range opts {
        opt(srv)
    }
    return srv
}

type Option func(*Server)

func WithMaxConns(n int) Option {
    return func(s *Server) {
        if n > 0 {
            s.maxConns = n
        }
    }
}

func WithLogger(l *log.Logger) Option {
    return func(s *Server) { s.logger = l }
}

func (s *Server) Host() string        { return s.host }
func (s *Server) Port() int           { return s.port }
func (s *Server) ActiveConns() int    { return s.activeConns }
func (s *Server) Uptime() time.Duration { return time.Since(s.startTime) }

func (s *Server) Shutdown() {
    close(s.shutdownChan)
    // graceful cleanup
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_type` inside `type_declaration`, `field_declaration` with exported (uppercase) `field_identifier`
- **Detection approach**: Find `type_declaration` nodes containing a `struct_type`. Within the `field_declaration_list`, identify `field_declaration` nodes where the `field_identifier` starts with an uppercase letter. Flag structs where more than half the fields are exported and the struct has 4+ fields. Heuristic: fields named with common internal-state patterns (`Count`, `Chan`, `Internal`, `Time`, `Connections`) are stronger signals.
- **S-expression query sketch**:
  ```scheme
  (type_declaration
    (type_spec
      name: (type_identifier) @struct_name
      type: (struct_type
        (field_declaration_list
          (field_declaration
            name: (field_identifier) @field_name)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `exported_struct_fields`
- **Severity**: warning
- **Confidence**: medium
