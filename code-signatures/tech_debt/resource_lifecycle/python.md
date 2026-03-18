# Resource Lifecycle -- Python

## Overview
Resources that are acquired but never properly released cause file descriptor exhaustion, connection pool starvation, and data corruption. In Python, the most common manifestations are files opened without context managers and database connections that are not closed in error paths.

## Why It's a Tech Debt Concern
Python's garbage collector will eventually close unreleased file handles, but the timing is non-deterministic and implementation-dependent (CPython uses reference counting, but PyPy does not). In long-running services, leaked file descriptors accumulate until the OS limit is hit, causing cascading "Too many open files" errors. Database connections leaked in error paths exhaust connection pools, causing the entire application to block waiting for available connections -- a failure mode that typically surfaces only under production load.

## Applicability
- **Relevance**: high (file I/O and database access are ubiquitous)
- **Languages covered**: `.py`, `.pyi`
- **Frameworks/libraries**: SQLAlchemy, psycopg2, sqlite3, Django ORM, any DB-API 2.0 driver

---

## Pattern 1: File Opened Without Context Manager

### Description
Opening a file with `open()` and assigning it to a variable without using a `with` statement. If an exception occurs between `open()` and `.close()`, the file handle leaks. Even when `.close()` is called explicitly, exceptions in intermediate code can bypass the close call.

### Bad Code (Anti-pattern)
```python
# Manual open/close -- exception between them leaks the handle
def read_config(path):
    f = open(path, 'r')
    data = json.load(f)  # If this raises, f is never closed
    f.close()
    return data

# Multiple files opened without context managers
def merge_files(input_paths, output_path):
    out = open(output_path, 'w')
    for path in input_paths:
        inp = open(path, 'r')
        out.write(inp.read())
        inp.close()
    out.close()

# open() return value used inline but never closed
def count_lines(path):
    lines = open(path).readlines()
    return len(lines)
```

### Good Code (Fix)
```python
# Context manager ensures close on any exit path
def read_config(path):
    with open(path, 'r') as f:
        return json.load(f)

# Nested context managers for multiple files
def merge_files(input_paths, output_path):
    with open(output_path, 'w') as out:
        for path in input_paths:
            with open(path, 'r') as inp:
                out.write(inp.read())

# Context manager for inline usage
def count_lines(path):
    with open(path) as f:
        return sum(1 for _ in f)
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment`, `call`, `identifier`, `with_statement`
- **Detection approach**: Find `call` nodes where the function is the identifier `open` (or `io.open`, `codecs.open`). Check if the call is the value in an `assignment` statement (i.e., `f = open(...)`) rather than being used as the expression in a `with_clause` of a `with_statement`. Flag assignments where `open()` is called outside a `with` context.
- **S-expression query sketch**:
  ```scheme
  ;; open() assigned to a variable (not in with statement)
  (assignment
    left: (identifier) @var_name
    right: (call
      function: (identifier) @func_name))

  ;; open() in with statement (safe -- do not flag)
  (with_statement
    (with_clause
      (with_item
        value: (call
          function: (identifier) @func_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `file_handle_leak`
- **Pattern name**: `open_without_context_manager`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Database Connection Not Closed in Finally

### Description
Acquiring a database connection and executing queries without ensuring the connection is closed in a `finally` block or `with` statement. If an exception occurs during query execution, the connection is leaked back to neither the pool nor the OS, eventually exhausting all available connections.

### Bad Code (Anti-pattern)
```python
# Connection leaked if execute() or fetchall() raises
def get_users(dsn):
    conn = psycopg2.connect(dsn)
    cursor = conn.cursor()
    cursor.execute("SELECT * FROM users")
    users = cursor.fetchall()
    cursor.close()
    conn.close()
    return users

# Connection leaked in error path
def insert_record(db_url, record):
    conn = sqlite3.connect(db_url)
    try:
        conn.execute("INSERT INTO records VALUES (?, ?)", record)
        conn.commit()
    except sqlite3.IntegrityError:
        log.error("Duplicate record: %s", record)
        return False
    # conn.close() never reached if IntegrityError is raised
    conn.close()
    return True

# SQLAlchemy engine -- session not closed on error
def update_user(engine, user_id, data):
    session = Session(engine)
    user = session.query(User).get(user_id)
    user.name = data['name']
    session.commit()
    session.close()
```

### Good Code (Fix)
```python
# Context manager ensures connection is closed
def get_users(dsn):
    with psycopg2.connect(dsn) as conn:
        with conn.cursor() as cursor:
            cursor.execute("SELECT * FROM users")
            return cursor.fetchall()

# try/finally ensures close on all paths
def insert_record(db_url, record):
    conn = sqlite3.connect(db_url)
    try:
        conn.execute("INSERT INTO records VALUES (?, ?)", record)
        conn.commit()
        return True
    except sqlite3.IntegrityError:
        log.error("Duplicate record: %s", record)
        return False
    finally:
        conn.close()

# SQLAlchemy session with context manager
def update_user(engine, user_id, data):
    with Session(engine) as session:
        user = session.query(User).get(user_id)
        user.name = data['name']
        session.commit()
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment`, `call`, `attribute`, `try_statement`, `finally_clause`
- **Detection approach**: Find `call` nodes invoking known connection constructors (`psycopg2.connect`, `sqlite3.connect`, `pymysql.connect`, `Session()`) assigned to a variable. Check if the variable's `.close()` method is called within a `finally_clause` of the enclosing `try_statement`, or if the call is used in a `with_statement`. Flag when close is called only in the happy path without `finally` protection.
- **S-expression query sketch**:
  ```scheme
  ;; Connection assigned to variable
  (assignment
    left: (identifier) @conn_var
    right: (call
      function: (attribute
        object: (identifier) @module
        attribute: (identifier) @connect_method)))

  ;; Close call in finally (safe pattern)
  (finally_clause
    body: (block
      (expression_statement
        (call
          function: (attribute
            object: (identifier) @conn_var
            attribute: (identifier) @close_method)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `file_handle_leak`
- **Pattern name**: `connection_not_closed_in_finally`
- **Severity**: warning
- **Confidence**: high
