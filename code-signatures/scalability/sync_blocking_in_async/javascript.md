# Sync Blocking in Async -- JavaScript

## Overview
Synchronous blocking patterns in JavaScript async contexts occur when blocking APIs like `*Sync()` Node.js methods, synchronous XHR, or tight loops are used inside `async function` bodies, blocking the event loop and preventing other tasks from executing.

## Why It's a Scalability Concern
JavaScript is single-threaded — blocking the event loop prevents all concurrent request handling, timer callbacks, and I/O completions. A single `readFileSync()` in an async Express handler blocks every other request until it completes. Under load, this causes request queuing, timeout cascades, and effectively serializes all server concurrency.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Node.js fs, child_process, crypto, Express, Koa, Fastify

---

## Pattern 1: Node.js *Sync Methods in Async Function

### Description
Using synchronous Node.js APIs like `readFileSync`, `writeFileSync`, `execSync`, `mkdirSync` inside an `async function` or a function containing `await` expressions.

### Bad Code (Anti-pattern)
```typescript
async function processUpload(req: Request, res: Response) {
  const data = req.body;
  const config = fs.readFileSync('/etc/app/config.json', 'utf-8');
  const parsed = JSON.parse(config);
  await db.save({ ...data, ...parsed });
  res.json({ ok: true });
}
```

### Good Code (Fix)
```typescript
async function processUpload(req: Request, res: Response) {
  const data = req.body;
  const config = await fs.promises.readFile('/etc/app/config.json', 'utf-8');
  const parsed = JSON.parse(config);
  await db.save({ ...data, ...parsed });
  res.json({ ok: true });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration` (with `async` keyword), `arrow_function`, `call_expression`, `identifier`, `member_expression`
- **Detection approach**: Find `call_expression` nodes where the function name ends in `Sync` (e.g., `readFileSync`, `writeFileSync`, `execSync`, `accessSync`) that are nested inside a function with the `async` keyword. Check the function declaration for the `async` modifier.
- **S-expression query sketch**:
```scheme
(function_declaration
  "async"
  body: (statement_block
    (expression_statement
      (call_expression
        function: (member_expression
          property: (property_identifier) @method)))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `sync_api_in_async`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Synchronous XHR in Async Context

### Description
Using `XMLHttpRequest` with `open(method, url, false)` (third argument `false` = synchronous) inside an async function, which blocks the main thread or worker thread.

### Bad Code (Anti-pattern)
```javascript
async function fetchConfig() {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/api/config', false); // synchronous
  xhr.send();
  return JSON.parse(xhr.responseText);
}
```

### Good Code (Fix)
```javascript
async function fetchConfig() {
  const response = await fetch('/api/config');
  return response.json();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `call_expression`, `member_expression`, `false`
- **Detection approach**: Find `call_expression` calling `.open()` with three arguments where the third argument is `false`, inside an `async` function. The object should be an `XMLHttpRequest` instance.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    property: (property_identifier) @method)
  arguments: (arguments
    (string)
    (string)
    (false) @sync_flag))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `sync_xhr_in_async`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: Tight While Loop in Async Function

### Description
Using a busy-wait `while` loop (polling a condition without yielding) inside an async function, which blocks the event loop and prevents other microtasks from executing.

### Bad Code (Anti-pattern)
```javascript
async function waitForReady(service) {
  while (!service.isReady()) {
    // busy wait — blocks event loop
  }
  return service.getData();
}
```

### Good Code (Fix)
```javascript
async function waitForReady(service) {
  while (!service.isReady()) {
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  return service.getData();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `while_statement`, `statement_block`
- **Detection approach**: Find `while_statement` inside an `async` function whose body (`statement_block`) contains no `await_expression` and no `call_expression` to yielding functions. An empty or trivially-bodied while loop in async context is the clearest signal.
- **S-expression query sketch**:
```scheme
(function_declaration
  "async"
  body: (statement_block
    (while_statement
      body: (statement_block) @loop_body)))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `busy_wait_in_async`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: Blocking prompt() in Async Function

### Description
Using `prompt()`, `alert()`, or `confirm()` inside an async function in browser contexts, which blocks the entire UI thread.

### Bad Code (Anti-pattern)
```javascript
async function submitForm(data) {
  const confirmed = prompt('Enter confirmation code:');
  if (confirmed) {
    await api.submit({ ...data, code: confirmed });
  }
}
```

### Good Code (Fix)
```javascript
async function submitForm(data, confirmationCode) {
  if (confirmationCode) {
    await api.submit({ ...data, code: confirmationCode });
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `call_expression`, `identifier`
- **Detection approach**: Find `call_expression` where the function is an `identifier` named `prompt`, `alert`, or `confirm` inside an `async` function.
- **S-expression query sketch**:
```scheme
(function_declaration
  "async"
  body: (statement_block
    (expression_statement
      (call_expression
        function: (identifier) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_dialog_in_async`
- **Severity**: info
- **Confidence**: high
