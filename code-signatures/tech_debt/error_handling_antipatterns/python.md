# Error Handling Anti-patterns -- Python

## Overview
Errors that are silently swallowed, broadly caught, or ignored make debugging impossible and hide real failures. In Python, bare `except` clauses, silent `pass` in exception handlers, and overly broad exception catching are the most common anti-patterns.

## Why It's a Tech Debt Concern
Bare `except` and `except Exception` catch everything including `SystemExit`, `KeyboardInterrupt`, and `GeneratorExit`, making it impossible to gracefully shut down processes or interrupt runaway code. Silent `pass` in except blocks means errors vanish without a trace, and bugs can persist for months before manifesting as mysterious downstream failures. Broad exception catching hides the specific failure mode, forcing developers to add speculative logging instead of handling known error types.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py`, `.pyi`

---

## Pattern 1: Bare Except

### Description
Using `except:` without specifying an exception type, or `except Exception:` which catches everything including `SystemExit` and `KeyboardInterrupt`. This prevents the process from being interrupted and masks critical errors.

### Bad Code (Anti-pattern)
```python
# Bare except -- catches everything including KeyboardInterrupt
try:
    result = perform_critical_operation()
except:
    log.error("Something failed")
    result = default_value

# except Exception -- still too broad
try:
    data = fetch_from_api(url)
except Exception:
    data = cached_data
```

### Good Code (Fix)
```python
# Catch specific exceptions
try:
    result = perform_critical_operation()
except (ConnectionError, TimeoutError) as e:
    log.error("Operation failed: %s", e)
    result = default_value
except ValueError as e:
    log.error("Invalid data: %s", e)
    raise

# Specific exception with fallback
try:
    data = fetch_from_api(url)
except requests.RequestException as e:
    log.warning("API unavailable, using cache: %s", e)
    data = cached_data
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `except_clause`
- **Detection approach**: Find `except_clause` nodes that have no exception type argument (bare `except:`). Also flag `except_clause` nodes where the exception type is `Exception` or `BaseException` (identifier node with those specific names).
- **S-expression query sketch**:
```scheme
;; Bare except (no type specified)
(except_clause) @bare_except

;; except Exception
(except_clause
  (identifier) @exception_type)
```

### Pipeline Mapping
- **Pipeline name**: `bare_except`
- **Pattern name**: `bare_except_clause`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Silent Pass in Except

### Description
An `except` block that contains only `pass` (or only a comment and `pass`), completely silencing the error. The failure is caught and discarded with no logging, no re-raise, and no alternative handling.

### Bad Code (Anti-pattern)
```python
try:
    config = load_config(path)
except FileNotFoundError:
    pass

try:
    send_notification(user, message)
except Exception:
    pass  # Don't let notification failures break the flow

for item in items:
    try:
        process(item)
    except ValueError:
        pass
```

### Good Code (Fix)
```python
try:
    config = load_config(path)
except FileNotFoundError:
    log.info("Config file not found at %s, using defaults", path)
    config = default_config()

try:
    send_notification(user, message)
except NotificationError as e:
    log.warning("Notification failed for user %s: %s", user.id, e)
    metrics.increment("notification_failures")

for item in items:
    try:
        process(item)
    except ValueError as e:
        log.warning("Skipping invalid item %s: %s", item.id, e)
        failed_items.append((item, e))
```

### Tree-sitter Detection Strategy
- **Target node types**: `except_clause`, `block`, `pass_statement`
- **Detection approach**: Find `except_clause` nodes whose body block contains only a `pass_statement` (and optionally `comment` nodes). Count the non-comment children of the except body block; if the only substantive statement is `pass`, flag it.
- **S-expression query sketch**:
```scheme
(except_clause
  body: (block
    (pass_statement) @silent_pass))
```

### Pipeline Mapping
- **Pipeline name**: `bare_except`
- **Pattern name**: `silent_pass_in_except`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Broad Exception Catching

### Description
Using `except Exception as e` when specific exception types should be caught. This catches every non-system exception including `TypeError`, `AttributeError`, `NameError`, and other bugs that should crash loudly rather than being handled as expected errors.

### Bad Code (Anti-pattern)
```python
def get_user_age(user_data):
    try:
        return int(user_data["age"])
    except Exception as e:
        logger.error("Could not get age: %s", e)
        return None

def connect_and_query(query):
    try:
        conn = database.connect()
        result = conn.execute(query)
        return result.fetchall()
    except Exception as e:
        logger.error("Query failed: %s", e)
        return []
```

### Good Code (Fix)
```python
def get_user_age(user_data):
    try:
        return int(user_data["age"])
    except KeyError:
        logger.warning("Missing 'age' field in user data")
        return None
    except (ValueError, TypeError) as e:
        logger.warning("Invalid age value: %s", e)
        return None

def connect_and_query(query):
    try:
        conn = database.connect()
        result = conn.execute(query)
        return result.fetchall()
    except ConnectionError as e:
        logger.error("Database connection failed: %s", e)
        raise
    except database.QueryError as e:
        logger.error("Query execution failed: %s", e)
        return []
```

### Tree-sitter Detection Strategy
- **Target node types**: `except_clause`, `identifier`, `as_pattern`
- **Detection approach**: Find `except_clause` nodes where the exception type is the identifier `Exception` (not a more specific type). Distinguish from bare except by checking that the `as` clause with a variable binding is present. Flag when `Exception` is the only type in the clause.
- **S-expression query sketch**:
```scheme
(except_clause
  (as_pattern
    (identifier) @exception_type
    alias: (as_pattern_target
      (identifier) @error_var)))
```

### Pipeline Mapping
- **Pipeline name**: `bare_except`
- **Pattern name**: `broad_exception_catch`
- **Severity**: info
- **Confidence**: medium
