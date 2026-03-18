# Comment Ratio -- JavaScript

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```javascript
function processOrder(order, inventory, discounts) {
  let total = 0;
  for (const item of order.items) {
    const stock = inventory[item.sku];
    if (!stock || stock.quantity < item.quantity) {
      if (stock && stock.backorderAllowed) {
        total += item.price * item.quantity * 1.15;
      } else {
        throw new Error(`Out of stock: ${item.sku}`);
      }
    } else {
      let price = item.price * item.quantity;
      for (const discount of discounts) {
        if (discount.sku === item.sku && discount.minQty <= item.quantity) {
          price *= (1 - discount.rate);
          break;
        } else if (discount.sku === '*' && total > discount.threshold) {
          price *= (1 - discount.rate);
        }
      }
      total += price;
      inventory[item.sku].quantity -= item.quantity;
    }
  }
  if (order.membership === 'premium') {
    total *= 0.95;
  } else if (order.membership === 'vip' && total > 500) {
    total *= 0.90;
  }
  return Math.round(total * 100) / 100;
}
```

### Good Code (Fix)
```javascript
function processOrder(order, inventory, discounts) {
  let total = 0;
  for (const item of order.items) {
    const stock = inventory[item.sku];

    if (!stock || stock.quantity < item.quantity) {
      // Backorder-eligible items incur a 15% surcharge to cover delayed fulfillment costs
      if (stock && stock.backorderAllowed) {
        total += item.price * item.quantity * 1.15;
      } else {
        throw new Error(`Out of stock: ${item.sku}`);
      }
    } else {
      let price = item.price * item.quantity;

      // Apply first matching SKU-specific discount, but global discounts ('*')
      // stack — they apply based on running total exceeding the threshold
      for (const discount of discounts) {
        if (discount.sku === item.sku && discount.minQty <= item.quantity) {
          price *= (1 - discount.rate);
          break;
        } else if (discount.sku === '*' && total > discount.threshold) {
          price *= (1 - discount.rate);
        }
      }
      total += price;
      inventory[item.sku].quantity -= item.quantity;
    }
  }

  // Membership tiers: premium gets flat 5%, VIP gets 10% only on large orders
  if (order.membership === 'premium') {
    total *= 0.95;
  } else if (order.membership === 'vip' && total > 500) {
    total *= 0.90;
  }

  return Math.round(total * 100) / 100;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `function`, `arrow_function`, `method_definition` for function bodies; `comment` for all comment types (`//`, `/* */`, `/** */`)
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_declaration
    body: (statement_block) @function.body)

  (comment) @comment

  ;; Also match arrow functions and methods
  (arrow_function
    body: (statement_block) @function.body)

  (method_definition
    body: (statement_block) @function.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```javascript
function calculateShipping(weight, destination) {
  // Declare the base rate
  let baseRate = 5.99;

  // Check if weight is greater than 50
  if (weight > 50) {
    // Multiply base rate by 2
    baseRate *= 2;
  }

  // Check if destination is international
  if (destination === 'international') {
    // Add 15 to base rate
    baseRate += 15;
  }

  // Return the base rate
  return baseRate;
}
```

### Good Code (Fix)
```javascript
function calculateShipping(weight, destination) {
  let baseRate = 5.99;

  // Heavy packages require freight carrier instead of standard parcel
  if (weight > 50) {
    baseRate *= 2;
  }

  // International surcharge covers customs brokerage fees
  if (destination === 'international') {
    baseRate += 15;
  }

  return baseRate;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `expression_statement`, `variable_declaration`, `return_statement`, `if_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (statement_block
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
