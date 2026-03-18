# Magic Values -- JavaScript

## Overview
Magic values are unexplained numeric literals, string constants, or boolean flags embedded directly in code without named constants or documentation. They make code hard to understand and modify.

## Why It's a Tech Debt Concern
Without a named constant, a reader cannot know what 86400 means (seconds in a day), why 3 retries were chosen, or what status code 42 represents. Changes require finding and updating every occurrence. Wrong updates cause subtle bugs.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: N/A

---

## Pattern 1: Magic Numbers in Logic

### Description
Numeric literals (other than 0, 1, -1) used in conditions, calculations, or assignments without a named constant explaining their meaning.

### Bad Code (Anti-pattern)
```javascript
function processRequest(data) {
  if (data.length > 1024) {
    return { status: 413 };
  }
  for (let i = 0; i < 3; i++) {
    setTimeout(() => retry(data), 86400 * 1000);
  }
  if (response.status === 200 || response.status === 404) {
    cache.set(key, data, 3600);
  }
}
```

### Good Code (Fix)
```javascript
const MAX_PAYLOAD_SIZE = 1024;
const MAX_RETRIES = 3;
const SECONDS_PER_DAY = 86400;
const MS_PER_SECOND = 1000;
const HTTP_OK = 200;
const HTTP_NOT_FOUND = 404;
const CACHE_TTL_SECONDS = 3600;

function processRequest(data) {
  if (data.length > MAX_PAYLOAD_SIZE) {
    return { status: HTTP_PAYLOAD_TOO_LARGE };
  }
  for (let i = 0; i < MAX_RETRIES; i++) {
    setTimeout(() => retry(data), SECONDS_PER_DAY * MS_PER_SECOND);
  }
  if (response.status === HTTP_OK || response.status === HTTP_NOT_FOUND) {
    cache.set(key, data, CACHE_TTL_SECONDS);
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `number` (excludes 0, 1, -1)
- **Detection approach**: Find `number` nodes in expressions (excluding array indices of 0/1, loop bounds of 0, and constant definitions). Flag literals that are not part of a `const` `lexical_declaration` and are not 0, 1, or -1.
- **S-expression query sketch**:
```scheme
(number) @number
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
```javascript
function handleUser(user) {
  if (user.role === "admin") {
    grantAccess("dashboard");
  }
  if (user.status === "active" || user.status === "pending") {
    notify(user);
  }
  const dbUrl = config["database_url"];
  const mode = settings["production"];
}
```

### Good Code (Fix)
```javascript
const Role = { ADMIN: "admin" };
const Status = { ACTIVE: "active", PENDING: "pending" };
const ConfigKeys = { DATABASE_URL: "database_url", MODE: "production" };

function handleUser(user) {
  if (user.role === Role.ADMIN) {
    grantAccess("dashboard");
  }
  if (user.status === Status.ACTIVE || user.status === Status.PENDING) {
    notify(user);
  }
  const dbUrl = config[ConfigKeys.DATABASE_URL];
  const mode = settings[ConfigKeys.MODE];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `string` in `binary_expression` (equality checks) or `subscript_expression` (bracket access)
- **Detection approach**: Find `string` nodes used in equality comparisons (`===`, `==`, `!==`, `!=`) or as computed property keys in `subscript_expression`. Exclude logging strings, error messages, and SQL queries. Flag repeated identical strings across a function or file.
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
