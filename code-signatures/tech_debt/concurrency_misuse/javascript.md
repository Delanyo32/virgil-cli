# Concurrency Misuse -- JavaScript

## Overview
Concurrency misuse in JavaScript manifests as deeply nested callback chains instead of modern async/await patterns, and as unhandled errors in concurrent promise operations. These anti-patterns make code harder to reason about, debug, and maintain.

## Why It's a Tech Debt Concern
Callback hell creates deeply indented, hard-to-follow control flow that is extremely difficult to refactor or extend. Error handling in nested callbacks is fragile — a single missed error parameter silently swallows failures. Unhandled rejections in `Promise.all` can crash Node.js processes and leave the application in an inconsistent state when one of several concurrent operations fails without cleanup.

## Applicability
- **Relevance**: high
- **Languages covered**: `.js`, `.jsx`
- **Frameworks/libraries**: Node.js (fs, http callbacks), Express (middleware chains), any callback-based API

---

## Pattern 1: Callback Hell

### Description
Deeply nested callbacks (3+ levels) used for sequential asynchronous operations instead of promises or async/await. Each level of nesting adds indentation and makes error handling progressively harder to manage correctly.

### Bad Code (Anti-pattern)
```javascript
function processOrder(orderId) {
  db.getOrder(orderId, function (err, order) {
    if (err) { console.error(err); return; }
    db.getUser(order.userId, function (err, user) {
      if (err) { console.error(err); return; }
      paymentService.charge(user.paymentMethod, order.total, function (err, receipt) {
        if (err) { console.error(err); return; }
        emailService.send(user.email, receipt, function (err) {
          if (err) { console.error(err); return; }
          db.updateOrder(orderId, { status: 'completed' }, function (err) {
            if (err) { console.error(err); return; }
            console.log('Order processed');
          });
        });
      });
    });
  });
}
```

### Good Code (Fix)
```javascript
async function processOrder(orderId) {
  const order = await db.getOrder(orderId);
  const user = await db.getUser(order.userId);
  const receipt = await paymentService.charge(user.paymentMethod, order.total);
  await emailService.send(user.email, receipt);
  await db.updateOrder(orderId, { status: 'completed' });
  console.log('Order processed');
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `arrow_function`, `function` (function expression)
- **Detection approach**: Find `call_expression` nodes whose last argument is a `function` or `arrow_function` that itself contains a `call_expression` with a function/arrow_function argument, nested 3+ levels deep. Walk the tree from each callback argument to count nesting depth.
- **S-expression query sketch**:
```scheme
(call_expression
  arguments: (arguments
    (arrow_function
      body: (statement_block
        (expression_statement
          (call_expression
            arguments: (arguments
              (arrow_function) @inner_callback)))))))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `callback_hell`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Unhandled Promise Rejection in Concurrent Operations

### Description
Using `Promise.all()` without wrapping it in try/catch or attaching a `.catch()` handler. When one promise rejects, the other promises continue executing but their results are lost, and the rejection may go unhandled — causing resource leaks, inconsistent state, or process crashes.

### Bad Code (Anti-pattern)
```javascript
async function syncAllData(userIds) {
  const results = await Promise.all(
    userIds.map(id => fetchAndSaveUser(id))
  );
  return results;
}

// Even worse: fire-and-forget concurrent operations
function refreshCaches() {
  Promise.all([
    cache.refresh('users'),
    cache.refresh('products'),
    cache.refresh('orders'),
  ]);
  // No .catch(), no await — rejections silently lost
}
```

### Good Code (Fix)
```javascript
async function syncAllData(userIds) {
  const results = await Promise.allSettled(
    userIds.map(id => fetchAndSaveUser(id))
  );
  const failures = results.filter(r => r.status === 'rejected');
  if (failures.length > 0) {
    logger.error('Some syncs failed', { failures: failures.map(f => f.reason) });
  }
  return results.filter(r => r.status === 'fulfilled').map(r => r.value);
}

async function refreshCaches() {
  try {
    await Promise.all([
      cache.refresh('users'),
      cache.refresh('products'),
      cache.refresh('orders'),
    ]);
  } catch (error) {
    logger.error('Cache refresh failed', error);
    throw error;
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `await_expression`
- **Detection approach**: Find `call_expression` nodes where the function is `Promise.all` (via `member_expression` with object `Promise` and property `all`). Flag when (1) the call is not inside a `try_statement` and has no `.catch()` chained, or (2) the call is an `expression_statement` (fire-and-forget) without `await` and without `.catch()`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @promise_obj
    property: (property_identifier) @all_method))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `unhandled_promise_all`
- **Severity**: warning
- **Confidence**: high
