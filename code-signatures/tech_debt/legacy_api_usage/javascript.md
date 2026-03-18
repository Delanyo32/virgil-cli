# Legacy API Usage -- JavaScript

## Overview
Legacy API usage refers to relying on outdated language features, deprecated APIs, or obsolete patterns when modern, safer, and more performant alternatives exist. In JavaScript, common examples include using `var` instead of block-scoped declarations, leaving `console.log` statements in production code, and using loose equality operators.

## Why It's a Tech Debt Concern
Legacy patterns accumulate silently because they still "work" -- `var` hoists unexpectedly, `console.log` leaks sensitive data to browser consoles, and `==` performs type coercion that causes subtle bugs. New team members unfamiliar with legacy quirks introduce regressions. Linters flag these issues but are often ignored when legacy code is grandfathered in, creating a broken-windows effect across the codebase.

## Applicability
- **Relevance**: high (these patterns are extremely common in codebases with pre-ES6 roots)
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: N/A (language-level patterns)

---

## Pattern 1: var Instead of let/const

### Description
Using `var` for variable declarations instead of `let` or `const`. `var` is function-scoped (not block-scoped), hoists to the top of the enclosing function, and allows re-declaration -- all of which lead to subtle bugs especially inside loops, conditionals, and closures.

### Bad Code (Anti-pattern)
```javascript
function processUsers(users) {
  for (var i = 0; i < users.length; i++) {
    var user = users[i];
    var name = user.name;
    var email = user.email;

    if (user.active) {
      var status = 'active';
      var lastLogin = user.lastLogin;
    }

    // `status` and `lastLogin` are accessible here due to hoisting,
    // even when user.active is false -- they are `undefined`
    console.log(name, status);
  }

  // `i`, `user`, `name`, `email`, `status`, `lastLogin` all leak here
  console.log('Processed', i, 'users');
}

function createHandlers(items) {
  var handlers = [];
  for (var i = 0; i < items.length; i++) {
    handlers.push(function() {
      // Bug: all handlers capture the same `i` (final value)
      return items[i];
    });
  }
  return handlers;
}
```

### Good Code (Fix)
```javascript
function processUsers(users) {
  for (let i = 0; i < users.length; i++) {
    const user = users[i];
    const name = user.name;
    const email = user.email;

    if (user.active) {
      const status = 'active';
      const lastLogin = user.lastLogin;
      console.log(name, status);
    }
  }

  console.log('Processed', users.length, 'users');
}

function createHandlers(items) {
  const handlers = [];
  for (let i = 0; i < items.length; i++) {
    handlers.push(function() {
      // Each iteration gets its own `i` binding
      return items[i];
    });
  }
  return handlers;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `variable_declaration` with `var` keyword
- **Detection approach**: Find all `variable_declaration` nodes whose declaration kind is `var`. Every occurrence is a candidate for replacement with `let` or `const`. Higher confidence when `var` appears inside a `for_statement`, `if_statement`, or block scope where hoisting causes semantic differences.
- **S-expression query sketch**:
```scheme
(variable_declaration
  kind: "var"
  (variable_declarator
    name: (identifier) @var_name))
```

### Pipeline Mapping
- **Pipeline name**: `var_usage`
- **Pattern name**: `var_instead_of_let_const`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: console.log Left in Production Code

### Description
`console.log`, `console.warn`, `console.error`, and `console.debug` calls left in production source code. These leak internal state to browser developer tools, add noise to server logs, and can expose sensitive information like tokens, passwords, or user data.

### Bad Code (Anti-pattern)
```javascript
async function authenticateUser(credentials) {
  console.log('Authenticating user:', credentials);
  const user = await db.findUser(credentials.email);

  console.log('Found user:', user);
  console.log('Password hash:', user.passwordHash);

  const valid = await bcrypt.compare(credentials.password, user.passwordHash);
  console.log('Password valid:', valid);

  if (!valid) {
    console.error('Authentication failed for:', credentials.email);
    throw new AuthError('Invalid credentials');
  }

  const token = generateToken(user);
  console.log('Generated token:', token);
  return { user, token };
}

function calculatePrice(items, discount) {
  console.log('Items:', JSON.stringify(items));
  const subtotal = items.reduce((sum, item) => sum + item.price, 0);
  console.log('Subtotal:', subtotal);
  const total = subtotal * (1 - discount);
  console.log('Total after discount:', total);
  return total;
}
```

### Good Code (Fix)
```javascript
import { logger } from './logger';

async function authenticateUser(credentials) {
  logger.debug('Authenticating user', { email: credentials.email });
  const user = await db.findUser(credentials.email);

  const valid = await bcrypt.compare(credentials.password, user.passwordHash);

  if (!valid) {
    logger.warn('Authentication failed', { email: credentials.email });
    throw new AuthError('Invalid credentials');
  }

  const token = generateToken(user);
  logger.info('User authenticated', { userId: user.id });
  return { user, token };
}

function calculatePrice(items, discount) {
  logger.debug('Calculating price', { itemCount: items.length, discount });
  const subtotal = items.reduce((sum, item) => sum + item.price, 0);
  const total = subtotal * (1 - discount);
  logger.debug('Price calculated', { subtotal, total });
  return total;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`
- **Detection approach**: Find `call_expression` nodes where the function is a `member_expression` with object `console` and property matching `log`, `warn`, `error`, `debug`, `info`, or `trace`. Exclude test files (`*.test.js`, `*.spec.js`) and development-only modules.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  (#eq? @obj "console")
  (#match? @method "^(log|warn|error|debug|info|trace)$"))
```

### Pipeline Mapping
- **Pipeline name**: `console_log_in_prod`
- **Pattern name**: `console_statement_in_source`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 3: Loose Equality (== instead of ===)

### Description
Using the abstract equality operator `==` (or `!=`) instead of the strict equality operator `===` (or `!==`). The abstract operator performs type coercion, leading to surprising results like `0 == ""` being `true`, `null == undefined` being `true`, and `[] == false` being `true`.

### Bad Code (Anti-pattern)
```javascript
function handleResponse(response) {
  if (response.status == 200) {
    // Works, but allows "200" == 200 to pass
    processSuccess(response);
  }

  if (response.data == null) {
    // Matches both null and undefined (sometimes intentional, usually not)
    return defaultData;
  }

  if (response.count == 0) {
    // "0" == 0 is true, "" == 0 is also true
    showEmptyState();
  }

  if (user.role != 'admin') {
    // Loose inequality -- same coercion issues
    restrictAccess();
  }
}

function isValid(value) {
  if (value == true) {
    return 'truthy';
  }
  if (value == false) {
    return 'falsy';
  }
}
```

### Good Code (Fix)
```javascript
function handleResponse(response) {
  if (response.status === 200) {
    processSuccess(response);
  }

  if (response.data === null || response.data === undefined) {
    return defaultData;
  }

  if (response.count === 0) {
    showEmptyState();
  }

  if (user.role !== 'admin') {
    restrictAccess();
  }
}

function isValid(value) {
  if (value === true) {
    return 'truthy';
  }
  if (value === false) {
    return 'falsy';
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression` with `==` or `!=` operator
- **Detection approach**: Find `binary_expression` nodes whose operator is `==` or `!=` (not `===` or `!==`). Exclude comparisons against `null` if the project intentionally uses `== null` to check for both `null` and `undefined` (configurable exclusion). Flag all other loose equality uses.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (_) @left
  operator: ["==" "!="]
  right: (_) @right)
```

### Pipeline Mapping
- **Pipeline name**: `loose_equality`
- **Pattern name**: `abstract_equality_operator`
- **Severity**: warning
- **Confidence**: high
