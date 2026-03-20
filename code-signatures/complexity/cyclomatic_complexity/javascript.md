# Cyclomatic Complexity -- JavaScript/TypeScript

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `else if`, `switch` cases, loops, ternary operators, and logical operators (`&&`, `||`, `??`). High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Every decision point introduces a new path that requires its own test case, so functions with high CC demand disproportionate testing effort. Developers struggle to mentally trace all branches, increasing the likelihood of missed edge cases and regressions. Empirical studies consistently show a strong correlation between cyclomatic complexity and defect density.

## Applicability
- **Relevance**: high
- **Languages covered**: `.ts`, `.tsx`, `.js`, `.jsx`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```typescript
function processOrder(order: Order): string {
  let result = "";
  if (order.status === "pending") {
    if (order.paymentMethod === "credit") {
      if (order.amount > 1000 || order.isVip) {
        result = "manual_review";
      } else if (order.amount > 500 && order.currency !== "USD") {
        result = "currency_review";
      } else {
        result = "auto_approve";
      }
    } else if (order.paymentMethod === "debit") {
      result = order.amount > 500 ? "limit_check" : "auto_approve";
    } else if (order.paymentMethod === "crypto") {
      result = "compliance_review";
    } else {
      result = "unknown_payment";
    }
  } else if (order.status === "processing") {
    if (order.shippingMethod === "express" && order.weight > 50) {
      result = "oversized_express";
    } else if (order.shippingMethod === "standard" || order.shippingMethod === "economy") {
      result = "normal_shipping";
    } else {
      result = "default_shipping";
    }
  } else if (order.status === "cancelled") {
    result = order.refundIssued ? "closed" : "pending_refund";
  } else {
    result = "unknown_status";
  }
  return result;
}
```

### Good Code (Fix)
```typescript
const paymentHandlers: Record<string, (order: Order) => string> = {
  credit: (order) => {
    if (order.amount > 1000 || order.isVip) return "manual_review";
    if (order.amount > 500 && order.currency !== "USD") return "currency_review";
    return "auto_approve";
  },
  debit: (order) => (order.amount > 500 ? "limit_check" : "auto_approve"),
  crypto: () => "compliance_review",
};

function resolveShipping(order: Order): string {
  if (order.shippingMethod === "express" && order.weight > 50) return "oversized_express";
  if (order.shippingMethod === "standard" || order.shippingMethod === "economy") return "normal_shipping";
  return "default_shipping";
}

function processOrder(order: Order): string {
  switch (order.status) {
    case "pending":
      return (paymentHandlers[order.paymentMethod] ?? (() => "unknown_payment"))(order);
    case "processing":
      return resolveShipping(order);
    case "cancelled":
      return order.refundIssued ? "closed" : "pending_refund";
    default:
      return "unknown_status";
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_clause`, `switch_case`, `for_statement`, `for_in_statement`, `while_statement`, `do_statement`, `ternary_expression`, `binary_expression` (with `&&`, `||`, `??`), `catch_clause`
- **Detection approach**: Count decision points within a function body (including arrow function bodies). Each `if`, `else if`, `case`, `for`, `for...of`, `while`, `do...while`, `&&`, `||`, `??`, `?:`, and `catch` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_declaration body: (statement_block) @fn_body) @fn
(method_definition body: (statement_block) @fn_body) @fn
(arrow_function body: (_) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(switch_case) @decision
(for_statement) @decision
(for_in_statement) @decision
(while_statement) @decision
(do_statement) @decision
(ternary_expression) @decision
(catch_clause) @decision
(binary_expression operator: ["&&" "||" "??"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or switch statements that compound complexity. Each nesting level multiplies cognitive load, making control flow extremely difficult to follow.

### Bad Code (Anti-pattern)
```typescript
function validateUser(user: User, context: Context): ValidationResult {
  if (user) {
    if (user.isActive) {
      if (context.requiresAuth) {
        if (user.token && user.token.isValid()) {
          if (user.roles.includes(context.requiredRole)) {
            if (!user.isBanned) {
              return { valid: true };
            } else {
              return { valid: false, reason: "banned" };
            }
          } else {
            return { valid: false, reason: "insufficient_role" };
          }
        } else {
          return { valid: false, reason: "invalid_token" };
        }
      } else {
        return { valid: true };
      }
    } else {
      return { valid: false, reason: "inactive" };
    }
  } else {
    return { valid: false, reason: "no_user" };
  }
}
```

### Good Code (Fix)
```typescript
function validateUser(user: User, context: Context): ValidationResult {
  if (!user) return { valid: false, reason: "no_user" };
  if (!user.isActive) return { valid: false, reason: "inactive" };
  if (!context.requiresAuth) return { valid: true };
  if (!user.token || !user.token.isValid()) return { valid: false, reason: "invalid_token" };
  if (!user.roles.includes(context.requiredRole)) return { valid: false, reason: "insufficient_role" };
  if (user.isBanned) return { valid: false, reason: "banned" };

  return { valid: true };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_clause` containing nested `if_statement`
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function boundary. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested if statements (3+ levels)
(if_statement
  consequence: (statement_block
    (if_statement
      consequence: (statement_block
        (if_statement) @deeply_nested))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
