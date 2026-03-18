# Race Conditions -- JavaScript

## Overview
Although JavaScript is single-threaded, its asynchronous execution model (Promises, async/await, callbacks) introduces race conditions when code checks a condition and then acts on it without atomicity. Between the check and the action, another asynchronous operation can interleave and invalidate the assumption, leading to incorrect behavior such as duplicate writes, stale reads, or security bypasses.

## Why It's a Security Concern
Check-then-act races in async JavaScript can lead to double-spending in payment flows, duplicate resource creation, file corruption when multiple async handlers write to the same resource, and authentication bypasses when session state changes between a permission check and the protected operation. In server-side Node.js, these races are exploitable under concurrent requests.

## Applicability
- **Relevance**: medium
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Node.js fs/promises, Express, Koa, any async middleware

---

## Pattern 1: Check-then-Act Race in Async Code

### Description
Checking a condition asynchronously (e.g., whether a file exists, whether a record is present in a database) and then acting on the result without ensuring atomicity. Between the `await` of the check and the `await` of the action, another concurrent request or event handler can change the state, invalidating the check.

### Bad Code (Anti-pattern)
```typescript
import { access, writeFile } from 'fs/promises';

async function safeWrite(filePath: string, data: string) {
  try {
    await access(filePath);
    // file exists -- skip writing
    return;
  } catch {
    // file does not exist -- write it
    // RACE: another request may create the file between check and write
    await writeFile(filePath, data);
  }
}
```

### Good Code (Fix)
```typescript
import { writeFile } from 'fs/promises';
import { constants } from 'fs';

async function safeWrite(filePath: string, data: string) {
  try {
    // wx flag: create exclusively -- fails atomically if file already exists
    await writeFile(filePath, data, { flag: 'wx' });
  } catch (err: any) {
    if (err.code === 'EEXIST') {
      return;  // file already exists -- safe to skip
    }
    throw err;
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `await_expression`, `call_expression`, `member_expression`, `try_statement`, `catch_clause`
- **Detection approach**: Find `await_expression` nodes calling `access`, `stat`, `existsSync`-like check functions inside a `try_statement`, followed by another `await_expression` calling `writeFile`, `mkdir`, or similar mutating fs operations in the same block or the `catch_clause`. The temporal gap between the two awaits indicates a check-then-act pattern.
- **S-expression query sketch**:
```scheme
(try_statement
  body: (statement_block
    (expression_statement
      (await_expression
        (call_expression
          function: (_) @check_func))))
  handler: (catch_clause
    body: (statement_block
      (expression_statement
        (await_expression
          (call_expression
            function: (_) @action_func))))))
```

### Pipeline Mapping
- **Pipeline name**: `race_conditions`
- **Pattern name**: `async_check_then_act`
- **Severity**: warning
- **Confidence**: medium
