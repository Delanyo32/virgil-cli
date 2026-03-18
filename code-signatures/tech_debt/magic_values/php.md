# Magic Values -- PHP

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```php
function processRequest(array $data): array {
    if (count($data) > 1024) {
        return ['status' => 413];
    }
    for ($i = 0; $i < 3; $i++) {
        sleep(86400);
    }
    if ($response->getStatusCode() === 200) {
        $cache->set($key, $data, 3600);
    } elseif ($response->getStatusCode() === 404) {
        return null;
    }
    return $data;
}
```

### Good Code (Fix)
```php
const MAX_PAYLOAD_SIZE = 1024;
const MAX_RETRIES = 3;
const SECONDS_PER_DAY = 86400;
const HTTP_OK = 200;
const HTTP_NOT_FOUND = 404;
const CACHE_TTL_SECONDS = 3600;

function processRequest(array $data): array {
    if (count($data) > MAX_PAYLOAD_SIZE) {
        return ['status' => HTTP_PAYLOAD_TOO_LARGE];
    }
    for ($i = 0; $i < MAX_RETRIES; $i++) {
        sleep(SECONDS_PER_DAY);
    }
    if ($response->getStatusCode() === HTTP_OK) {
        $cache->set($key, $data, CACHE_TTL_SECONDS);
    } elseif ($response->getStatusCode() === HTTP_NOT_FOUND) {
        return null;
    }
    return $data;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `integer`, `float` (excludes 0, 1, -1)
- **Detection approach**: Find `integer` and `float` nodes in expressions. Exclude literals inside `const_declaration` ancestors, `define()` call arguments, `enum_case` ancestors, and `class_constant_declaration` ancestors. Flag literals that are not 0, 1, or -1.
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
```php
function handleUser(User $user): void {
    if ($user->role === 'admin') {
        grantAccess('dashboard');
    }
    if ($user->status === 'active' || $user->status === 'pending') {
        notify($user);
    }
    $dbUrl = $config['database_url'];
    $mode = $settings['production'];
}
```

### Good Code (Fix)
```php
const ROLE_ADMIN = 'admin';
const STATUS_ACTIVE = 'active';
const STATUS_PENDING = 'pending';
const CONFIG_DATABASE_URL = 'database_url';
const CONFIG_MODE = 'production';

function handleUser(User $user): void {
    if ($user->role === ROLE_ADMIN) {
        grantAccess('dashboard');
    }
    if ($user->status === STATUS_ACTIVE || $user->status === STATUS_PENDING) {
        notify($user);
    }
    $dbUrl = $config[CONFIG_DATABASE_URL];
    $mode = $settings[CONFIG_MODE];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string` or `encapsed_string` in `binary_expression` (equality checks) or `subscript_expression` (array access)
- **Detection approach**: Find `string` nodes used in equality comparisons (`===`, `==`, `!==`, `!=`) or as array keys in `subscript_expression`. Exclude logging strings, SQL queries, and heredoc/nowdoc strings. Flag repeated identical strings across a function or class.
- **S-expression query sketch**:
```scheme
(binary_expression
  operator: ["===" "==" "!==" "!="]
  [left: (string) @string_lit
   right: (string) @string_lit])

(subscript_expression
  index: (string) @string_lit)
```

### Pipeline Mapping
- **Pipeline name**: `magic_numbers`
- **Pattern name**: `unexplained_string_literal`
- **Severity**: info
- **Confidence**: low
