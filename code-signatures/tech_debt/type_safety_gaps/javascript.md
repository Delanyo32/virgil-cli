# Type Safety Gaps -- JavaScript

## Overview
JavaScript's dynamic type system allows implicit coercions and untyped function signatures that introduce subtle bugs. Loose equality operators silently coerce operands, while missing JSDoc annotations on exported functions leave consumers guessing about expected types and return values.

## Why It's a Tech Debt Concern
Implicit type coercion via `==` leads to notoriously unintuitive behavior (`"" == 0` is `true`, `null == undefined` is `true`) that causes bugs which are difficult to diagnose. Missing type annotations on exported functions force consumers to read implementation code to understand the API contract, slow down IDE autocompletion, and prevent static analysis tools from catching type mismatches at development time.

## Applicability
- **Relevance**: high (JavaScript's dynamic typing makes these patterns extremely common)
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: All JavaScript codebases; ESLint `eqeqeq` rule targets Pattern 1

---

## Pattern 1: Implicit Type Coercion in Comparisons

### Description
Using the loose equality operator `==` or `!=` instead of strict equality `===` or `!==`. The loose operators perform type coercion before comparison, leading to unexpected truthy/falsy results that are a frequent source of bugs.

### Bad Code (Anti-pattern)
```javascript
function findUser(id) {
  if (id == null) {
    return null;
  }
  const user = users.find(u => u.id == id);
  if (user.role == 1) {
    grantAdmin(user);
  }
  if (user.age != "0") {
    validateAge(user);
  }
  return user;
}

function isActive(status) {
  return status == true;
}
```

### Good Code (Fix)
```javascript
function findUser(id) {
  if (id === null || id === undefined) {
    return null;
  }
  const user = users.find(u => u.id === id);
  if (user.role === 1) {
    grantAdmin(user);
  }
  if (user.age !== 0) {
    validateAge(user);
  }
  return user;
}

function isActive(status) {
  return status === true;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`
- **Detection approach**: Find `binary_expression` nodes whose operator is `==` or `!=`. These are the loose equality/inequality operators. Exclude comparisons against `null` if the project intentionally uses `== null` as an idiom for checking both `null` and `undefined`. Flag all other loose comparisons.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (_) @left
  operator: "==" @op
  right: (_) @right)

(binary_expression
  left: (_) @left
  operator: "!=" @op
  right: (_) @right)
```

### Pipeline Mapping
- **Pipeline name**: `loose_equality`
- **Pattern name**: `implicit_type_coercion`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Missing JSDoc Type Annotations on Exported Functions

### Description
Exported functions that lack JSDoc `@param` and `@returns` annotations leave the function's type contract undocumented. Without TypeScript or JSDoc types, callers have no way to know expected argument types or return values without reading the implementation.

### Bad Code (Anti-pattern)
```javascript
export function calculateTotal(items, taxRate, discount) {
  let total = 0;
  for (const item of items) {
    total += item.price * item.quantity;
  }
  total *= (1 + taxRate);
  total -= discount;
  return total;
}

export function formatUser(user, options) {
  const name = options.fullName ? `${user.first} ${user.last}` : user.first;
  return { name, email: user.email, active: user.active };
}
```

### Good Code (Fix)
```javascript
/**
 * Calculates the total price for a list of items with tax and discount.
 * @param {Array<{price: number, quantity: number}>} items - Line items
 * @param {number} taxRate - Tax rate as a decimal (e.g., 0.08 for 8%)
 * @param {number} discount - Flat discount amount to subtract
 * @returns {number} The final total after tax and discount
 */
export function calculateTotal(items, taxRate, discount) {
  let total = 0;
  for (const item of items) {
    total += item.price * item.quantity;
  }
  total *= (1 + taxRate);
  total -= discount;
  return total;
}

/**
 * Formats a user object for display.
 * @param {{first: string, last: string, email: string, active: boolean}} user
 * @param {{fullName?: boolean}} options - Formatting options
 * @returns {{name: string, email: string, active: boolean}}
 */
export function formatUser(user, options) {
  const name = options.fullName ? `${user.first} ${user.last}` : user.first;
  return { name, email: user.email, active: user.active };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `export_statement`, `function_declaration`, `comment`
- **Detection approach**: Find `export_statement` nodes containing a `function_declaration`. Check whether the preceding sibling is a `comment` node whose text starts with `/**` (a JSDoc block). If no JSDoc comment precedes the exported function, or if the JSDoc comment lacks `@param` or `@returns` tags, flag the function.
- **S-expression query sketch**:
```scheme
(export_statement
  (function_declaration
    name: (identifier) @func_name
    parameters: (formal_parameters) @params))
```

### Pipeline Mapping
- **Pipeline name**: `loose_equality`
- **Pattern name**: `missing_jsdoc_types`
- **Severity**: info
- **Confidence**: medium
