# Type Confusion -- JavaScript

## Overview
Type confusion vulnerabilities in JavaScript arise from the language's dynamic type system and implicit type coercion. Prototype pollution allows attackers to inject properties into `Object.prototype` via unguarded recursive merge or spread operations, affecting all objects in the application. Loose equality (`==`) performs type coercion that can bypass authentication checks when comparing values of different types.

## Why It's a Security Concern
Prototype pollution can lead to remote code execution, property injection, and denial of service by modifying the behavior of all objects in the runtime. Type coercion via `==` can cause authentication bypasses -- for example, `0 == ""` is `true`, and `null == undefined` is `true`, allowing attackers to craft inputs that pass security checks they should not. These patterns are especially dangerous in server-side Node.js applications where a single polluted prototype affects all requests.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: lodash (_.merge, _.defaultsDeep), jQuery ($.extend), express, any recursive object merge utility

---

## Pattern 1: Prototype Pollution via Object Merge

### Description
Using `Object.assign()`, spread operators, or recursive merge functions to copy properties from user-controlled objects without filtering dangerous keys like `__proto__`, `constructor`, or `prototype`. An attacker can supply JSON with `{"__proto__": {"isAdmin": true}}` to inject properties into `Object.prototype`, affecting every object in the application.

### Bad Code (Anti-pattern)
```javascript
function merge(target, source) {
  for (const key in source) {
    if (typeof source[key] === 'object' && source[key] !== null) {
      if (!target[key]) target[key] = {};
      merge(target[key], source[key]);
    } else {
      target[key] = source[key];
    }
  }
  return target;
}

// Attacker sends: {"__proto__": {"isAdmin": true}}
const userInput = JSON.parse(req.body);
const config = merge({}, userInput);

// Now every object has isAdmin === true
const user = {};
console.log(user.isAdmin); // true
```

### Good Code (Fix)
```javascript
function safeMerge(target, source) {
  for (const key in source) {
    if (key === '__proto__' || key === 'constructor' || key === 'prototype') {
      continue; // skip dangerous keys
    }
    if (Object.hasOwn(source, key)) {
      if (typeof source[key] === 'object' && source[key] !== null) {
        if (!target[key]) target[key] = {};
        safeMerge(target[key], source[key]);
      } else {
        target[key] = source[key];
      }
    }
  }
  return target;
}

// Or use Object.create(null) to avoid prototype chain entirely
const config = Object.create(null);
Object.assign(config, JSON.parse(req.body));
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_in_statement`, `member_expression`, `subscript_expression`, `call_expression`
- **Detection approach**: Find recursive functions that iterate over object keys using `for...in` and assign to `target[key]` without checking for `__proto__`, `constructor`, or `prototype`. Also flag `Object.assign()` calls where any source argument is derived from user input (e.g., `req.body`, `req.query`, parsed JSON). Look for the pattern of dynamic property assignment within a recursive function body.
- **S-expression query sketch**:
```scheme
(for_in_statement
  left: (_) @key_var
  right: (_) @source
  body: (statement_block
    (expression_statement
      (assignment_expression
        left: (subscript_expression
          object: (_) @target
          index: (identifier) @key_ref)))))
```

### Pipeline Mapping
- **Pipeline name**: `prototype_pollution`
- **Pattern name**: `object_merge_pollution`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Type Coercion Exploitation via Loose Equality

### Description
Using the loose equality operator (`==`) to compare values in security-sensitive contexts such as authentication checks, authorization gates, or input validation. JavaScript's `==` performs type coercion, which can cause unexpected truthy comparisons: `0 == ""` is `true`, `0 == false` is `true`, `null == undefined` is `true`, and `"0" == false` is `true`. Attackers can exploit these coercion rules to bypass checks.

### Bad Code (Anti-pattern)
```javascript
function authenticate(req, res) {
  const token = req.headers['x-auth-token'];
  const expectedToken = getExpectedToken(req.user);

  // Type coercion: if token is 0 and expectedToken is "" (empty string), this passes
  if (token == expectedToken) {
    return grantAccess(req, res);
  }
  return denyAccess(req, res);
}

function checkRole(user) {
  // If user.role is 0 (number), this incorrectly matches empty string
  if (user.role == "") {
    return "guest";
  }
  return user.role;
}
```

### Good Code (Fix)
```javascript
function authenticate(req, res) {
  const token = req.headers['x-auth-token'];
  const expectedToken = getExpectedToken(req.user);

  // Strict equality -- no type coercion
  if (typeof token === 'string' && token.length > 0 && token === expectedToken) {
    return grantAccess(req, res);
  }
  return denyAccess(req, res);
}

function checkRole(user) {
  if (user.role === "" || user.role === null || user.role === undefined) {
    return "guest";
  }
  return user.role;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `if_statement`, `identifier`
- **Detection approach**: Find `binary_expression` nodes with operator `==` or `!=` (not `===` or `!==`) inside `if_statement` conditions, especially when the comparison involves identifiers that suggest security context (e.g., `token`, `password`, `role`, `auth`, `secret`, `session`). Flag all loose equality usage in security-sensitive code paths.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (_) @lhs
  operator: "=="
  right: (_) @rhs)
```

### Pipeline Mapping
- **Pipeline name**: `type_confusion`
- **Pattern name**: `loose_equality_auth_bypass`
- **Severity**: error
- **Confidence**: medium
