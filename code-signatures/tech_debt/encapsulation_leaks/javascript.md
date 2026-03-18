# Encapsulation Leaks -- JavaScript

## Overview
Encapsulation leaks in JavaScript occur when internal state is exposed or mutated in ways that break module boundaries. Exported mutable variables allow any consumer to silently alter module state, while functions that mutate their arguments create hidden side effects that callers cannot anticipate. Both patterns make code harder to reason about and test.

## Why It's a Tech Debt Concern
Exported mutable state creates invisible coupling between modules — any file that imports the variable can change it, leading to unpredictable behavior and race conditions in concurrent environments. Argument mutation produces action-at-a-distance bugs where a function call silently changes an object the caller still holds a reference to, making debugging and refactoring hazardous. Both patterns resist safe parallel development because the true scope of a change is unknowable without tracing every consumer.

## Applicability
- **Relevance**: high (mutable module state and object mutation are extremely common in JS codebases)
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: Express (shared middleware state), React (mutated props/state), Redux (mutated action payloads), Node.js modules (cached singleton state)

---

## Pattern 1: Exported Mutable Module State

### Description
A module-level `let` or `var` variable is exported directly, allowing any importing module to read and modify shared state without encapsulation. This bypasses any validation, logging, or synchronization the owning module might need.

### Bad Code (Anti-pattern)
```javascript
// config.js
export let currentUser = null;
export let requestCount = 0;
export let featureFlags = { darkMode: false, betaAccess: false };
export let cache = {};

export function handleRequest(req) {
  requestCount++;
  currentUser = req.user;
  if (featureFlags.betaAccess) {
    // beta logic
  }
}

// consumer.js
import { currentUser, requestCount, featureFlags, cache } from './config.js';

// Any module can silently mutate shared state
featureFlags.darkMode = true;
cache['secret'] = sensitiveData;
requestCount = -1; // reset counter unexpectedly
```

### Good Code (Fix)
```javascript
// config.js
let currentUser = null;
let requestCount = 0;
let featureFlags = { darkMode: false, betaAccess: false };
const cache = new Map();

export function getCurrentUser() {
  return currentUser;
}

export function getRequestCount() {
  return requestCount;
}

export function getFeatureFlag(flag) {
  return featureFlags[flag] ?? false;
}

export function setFeatureFlag(flag, value) {
  if (!(flag in featureFlags)) throw new Error(`Unknown flag: ${flag}`);
  featureFlags[flag] = value;
}

export function handleRequest(req) {
  requestCount++;
  currentUser = req.user;
}

// consumer.js
import { getCurrentUser, getFeatureFlag, setFeatureFlag } from './config.js';

// Controlled access through functions
const user = getCurrentUser();
setFeatureFlag('darkMode', true);
```

### Tree-sitter Detection Strategy
- **Target node types**: `export_statement` containing `lexical_declaration` with `let` or `variable_declaration` with `var`
- **Detection approach**: Find `export_statement` nodes whose child is a `lexical_declaration` using `let` (or `variable_declaration` using `var`). These represent mutable exported bindings. Flag each occurrence. Exclude `const` exports (immutable binding, though object values can still be mutated).
- **S-expression query sketch**:
  ```scheme
  (export_statement
    (lexical_declaration
      kind: "let"
      (variable_declarator
        name: (identifier) @exported_var)))

  (export_statement
    (variable_declaration
      (variable_declarator
        name: (identifier) @exported_var)))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `exported_mutable_state`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Argument Mutation

### Description
A function modifies properties of objects passed as arguments instead of returning a new object with the desired changes. Callers cannot tell from the function signature that their data will be altered, leading to subtle bugs when the same object is shared across multiple call sites.

### Bad Code (Anti-pattern)
```javascript
function normalizeUser(user) {
  user.name = user.name.trim().toLowerCase();
  user.email = user.email.trim().toLowerCase();
  user.createdAt = new Date(user.createdAt);
  user.roles = [...new Set(user.roles)];
  delete user.password;
}

function enrichOrder(order, discounts) {
  order.subtotal = order.items.reduce((s, i) => s + i.price * i.qty, 0);
  order.discount = discounts[order.coupon] || 0;
  order.total = order.subtotal - order.discount;
  order.tax = order.total * 0.08;
  order.items.forEach(item => {
    item.lineTotal = item.price * item.qty;
  });
}

// caller
const user = await fetchUser(id);
normalizeUser(user); // user is silently mutated
sendWelcomeEmail(user); // receives mutated version — no password field

const order = await fetchOrder(id);
enrichOrder(order, discountMap); // order.items mutated too
```

### Good Code (Fix)
```javascript
function normalizeUser(user) {
  return {
    ...user,
    name: user.name.trim().toLowerCase(),
    email: user.email.trim().toLowerCase(),
    createdAt: new Date(user.createdAt),
    roles: [...new Set(user.roles)],
    password: undefined,
  };
}

function enrichOrder(order, discounts) {
  const subtotal = order.items.reduce((s, i) => s + i.price * i.qty, 0);
  const discount = discounts[order.coupon] || 0;
  const total = subtotal - discount;
  return {
    ...order,
    subtotal,
    discount,
    total,
    tax: total * 0.08,
    items: order.items.map(item => ({
      ...item,
      lineTotal: item.price * item.qty,
    })),
  };
}

// caller
const user = await fetchUser(id);
const normalizedUser = normalizeUser(user); // original untouched
sendWelcomeEmail(normalizedUser);

const order = await fetchOrder(id);
const enrichedOrder = enrichOrder(order, discountMap);
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `arrow_function`, `method_definition` — look for `assignment_expression` or `delete` targeting a parameter's property
- **Detection approach**: Identify function parameters from `formal_parameters`. Within the function body, find `assignment_expression` nodes where the left-hand side is a `member_expression` whose object matches a parameter name (e.g., `user.name = ...`). Also detect `delete` expressions targeting parameter properties. Flag functions that assign to 2+ parameter properties.
- **S-expression query sketch**:
  ```scheme
  (function_declaration
    parameters: (formal_parameters
      (identifier) @param_name)
    body: (statement_block
      (expression_statement
        (assignment_expression
          left: (member_expression
            object: (identifier) @mutated_obj)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `argument_mutation`
- **Pattern name**: `parameter_property_mutation`
- **Severity**: warning
- **Confidence**: medium
