# Dead Code -- JavaScript

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates bundle size, creates false positive search results, and can mislead developers into thinking features are still active. In JavaScript, dead code also bloats client-side payloads and slows down tree-shaking in bundlers.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Function/Method

### Description
A function or method defined but never called from anywhere in the codebase.

### Bad Code (Anti-pattern)
```javascript
// This helper was used by a feature that was deleted in v2.0
function formatLegacyDate(timestamp) {
  const d = new Date(timestamp * 1000);
  return `${d.getMonth() + 1}/${d.getDate()}/${d.getFullYear()}`;
}

// The only function actually called
function formatDate(isoString) {
  return new Date(isoString).toLocaleDateString();
}

module.exports = { formatDate };
```

### Good Code (Fix)
```javascript
function formatDate(isoString) {
  return new Date(isoString).toLocaleDateString();
}

module.exports = { formatDate };
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_definition`, `variable_declarator` (with arrow function value)
- **Detection approach**: Collect all function/method definitions and their names. Cross-reference with all `call_expression` nodes and `member_expression` accesses across the file and project. Functions with zero references (excluding test files) are candidates. Exclude exported functions, `main`, event handler registrations, and functions passed as callbacks.
- **S-expression query sketch**:
  ```scheme
  (function_declaration name: (identifier) @fn_name)
  (variable_declarator name: (identifier) @fn_name value: (arrow_function))
  (method_definition name: (property_identifier) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Break/Continue

### Description
Code statements that appear after an unconditional return, break, continue, or throw — they can never execute.

### Bad Code (Anti-pattern)
```javascript
function calculateDiscount(price, tier) {
  if (tier === 'premium') {
    return price * 0.8;
    console.log('Applied premium discount');  // unreachable
    logAnalytics('discount_applied', { tier }); // unreachable
  }
  throw new Error('Unknown tier');
  return price; // unreachable
}
```

### Good Code (Fix)
```javascript
function calculateDiscount(price, tier) {
  if (tier === 'premium') {
    return price * 0.8;
  }
  throw new Error('Unknown tier');
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `break_statement`, `continue_statement`, `throw_statement`
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `statement_block`. Those siblings are unreachable. Exclude statements inside nested blocks (if/else, try/catch) where the return is conditional.
- **S-expression query sketch**:
  ```scheme
  (statement_block
    (return_statement) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```javascript
function processOrder(order) {
  // function validateOrder(order) {
  //   if (!order.items || order.items.length === 0) {
  //     throw new Error('Empty order');
  //   }
  //   if (!order.customer) {
  //     throw new Error('No customer');
  //   }
  //   return true;
  // }
  //
  // const isValid = validateOrder(order);
  // if (!isValid) return null;

  return submitOrder(order);
}
```

### Good Code (Fix)
```javascript
function processOrder(order) {
  return submitOrder(order);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment`
- **Detection approach**: Find comment nodes whose content matches code syntax patterns (contains semicolons, braces, assignment operators, function keywords, `const`/`let`/`var` declarations). Flag blocks of 5+ consecutive comment lines that look like code rather than documentation.
- **S-expression query sketch**:
  ```scheme
  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
