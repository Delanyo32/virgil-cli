# Magic Values -- Python

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```python
def process_request(data):
    if len(data) > 1024:
        raise ValueError("Payload too large")
    for attempt in range(3):
        time.sleep(86400)
    if response.status_code == 200:
        cache.set(key, data, 3600)
    elif response.status_code == 404:
        return None
```

### Good Code (Fix)
```python
MAX_PAYLOAD_SIZE = 1024
MAX_RETRIES = 3
SECONDS_PER_DAY = 86400
HTTP_OK = 200
HTTP_NOT_FOUND = 404
CACHE_TTL_SECONDS = 3600

def process_request(data):
    if len(data) > MAX_PAYLOAD_SIZE:
        raise ValueError("Payload too large")
    for attempt in range(MAX_RETRIES):
        time.sleep(SECONDS_PER_DAY)
    if response.status_code == HTTP_OK:
        cache.set(key, data, CACHE_TTL_SECONDS)
    elif response.status_code == HTTP_NOT_FOUND:
        return None
```

### Tree-sitter Detection Strategy
- **Target node types**: `integer`, `float` (excludes 0, 1, -1)
- **Detection approach**: Find `integer` and `float` nodes in expressions. Exclude literals inside UPPER_SNAKE_CASE `assignment` targets (constant definitions), `keyword_argument` contexts, `default_parameter`/`typed_default_parameter` contexts, and `subscript` index positions. Flag literals that are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
[(integer) @number (float) @number]
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_numeric_literal`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Magic Strings

### Description
String literals used for comparisons, dictionary keys, or status values without named constants -- prone to typos and hard to refactor.

### Bad Code (Anti-pattern)
```python
def handle_user(user):
    if user.role == "admin":
        grant_access("dashboard")
    if user.status in ("active", "pending"):
        notify(user)
    db_url = config["database_url"]
    mode = settings["production"]
```

### Good Code (Fix)
```python
ROLE_ADMIN = "admin"
STATUS_ACTIVE = "active"
STATUS_PENDING = "pending"
CONFIG_DATABASE_URL = "database_url"
CONFIG_MODE = "production"

def handle_user(user):
    if user.role == ROLE_ADMIN:
        grant_access("dashboard")
    if user.status in (STATUS_ACTIVE, STATUS_PENDING):
        notify(user)
    db_url = config[CONFIG_DATABASE_URL]
    mode = settings[CONFIG_MODE]
```

### Tree-sitter Detection Strategy
- **Target node types**: `string` in `comparison_operator` (equality checks) or `subscript` (dictionary access)
- **Detection approach**: Find `string` nodes used in equality comparisons (`==`, `!=`, `in`, `not in`) or as dictionary keys in `subscript` expressions. Exclude logging strings, docstrings, error messages, and format strings. Flag repeated identical strings across a function or file.
- **S-expression query sketch**:
```scheme
(comparison_operator
  (string) @string_lit)

(subscript
  subscript: (string) @string_lit)
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
