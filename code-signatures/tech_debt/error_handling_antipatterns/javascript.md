# Error Handling Anti-patterns -- JavaScript

## Overview
Errors that are silently swallowed, broadly caught, or left unhandled make debugging impossible and hide real failures. In JavaScript, unhandled promise rejections, empty catch blocks, and swallowed errors are the most common manifestations.

## Why It's a Tech Debt Concern
Swallowed errors mask bugs that surface only in production under load, making root-cause analysis extremely difficult. Broad catch blocks hide important exceptions like TypeError or RangeError behind generic handling, allowing logic errors to persist undetected. Missing error handling on promises causes unexpected behavior and can crash Node.js processes with unhandled rejection warnings.

## Applicability
- **Relevance**: high
- **Languages covered**: `.js`, `.jsx`

---

## Pattern 1: Unhandled Promise Rejection

### Description
Using `.then()` without a corresponding `.catch()`, or calling an `async` function without `try/catch` or a `.catch()` handler, leaving promise rejections unhandled. In Node.js, unhandled rejections can terminate the process.

### Bad Code (Anti-pattern)
```javascript
// .then() without .catch()
fetch('/api/users')
  .then(response => response.json())
  .then(users => renderUsers(users));

// async function without try/catch
async function loadData() {
  const response = await fetch('/api/data');
  const data = await response.json();
  return data;
}

// Calling async function without catch
loadData().then(data => console.log(data));
```

### Good Code (Fix)
```javascript
// .then() with .catch()
fetch('/api/users')
  .then(response => response.json())
  .then(users => renderUsers(users))
  .catch(error => {
    logger.error('Failed to load users', error);
    showErrorBanner('Could not load users');
  });

// async function with try/catch
async function loadData() {
  try {
    const response = await fetch('/api/data');
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    return await response.json();
  } catch (error) {
    logger.error('Failed to load data', error);
    throw error;
  }
}

// Calling async function with catch
loadData()
  .then(data => console.log(data))
  .catch(error => handleError(error));
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `await_expression`
- **Detection approach**: Find call chains where `.then()` is called on a `call_expression` but no `.catch()` appears in the chain. For `await` expressions, check if the enclosing function body lacks a `try_statement` wrapping the `await`. Walk the member expression chain to verify absence of `.catch()`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (call_expression
      function: (member_expression
        property: (property_identifier) @then_call))
    property: (property_identifier) @next_call))
```

### Pipeline Mapping
- **Pipeline name**: `unhandled_promise`
- **Pattern name**: `then_without_catch`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Empty Catch Block

### Description
A `catch` block that contains no statements -- the error is caught and completely discarded, making it impossible to know that an error occurred. This silences failures and makes debugging extremely difficult.

### Bad Code (Anti-pattern)
```javascript
try {
  const data = JSON.parse(rawInput);
  processData(data);
} catch (e) {
}

try {
  await connectToDatabase();
} catch (error) {
  // TODO: handle this later
}
```

### Good Code (Fix)
```javascript
try {
  const data = JSON.parse(rawInput);
  processData(data);
} catch (e) {
  logger.error('Failed to parse input', { error: e, input: rawInput });
  throw new ValidationError('Invalid input format', { cause: e });
}

try {
  await connectToDatabase();
} catch (error) {
  logger.error('Database connection failed', error);
  throw new DatabaseError('Could not connect to database', { cause: error });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `statement_block`
- **Detection approach**: Find `catch_clause` nodes whose body (`statement_block`) has zero child statements. Also flag catch blocks containing only a comment (the block's children are all `comment` nodes).
- **S-expression query sketch**:
```scheme
(try_statement
  handler: (catch_clause
    parameter: (identifier) @error_var
    body: (statement_block) @catch_body))
```

### Pipeline Mapping
- **Pipeline name**: `unhandled_promise`
- **Pattern name**: `empty_catch_block`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Swallowed Error

### Description
A `catch` block that logs the error (e.g., `console.log`, `console.error`) but does not rethrow, return an error value, or otherwise propagate the failure. The calling code continues as if the operation succeeded.

### Bad Code (Anti-pattern)
```javascript
async function saveUser(user) {
  try {
    await db.insert('users', user);
    await sendWelcomeEmail(user.email);
    await notifyAdmin(user);
  } catch (error) {
    console.error('Something went wrong:', error);
  }
  // Execution continues -- caller has no idea the save failed
  return { success: true };
}

function parseConfig(path) {
  try {
    return JSON.parse(fs.readFileSync(path, 'utf-8'));
  } catch (e) {
    console.log('Config parse error', e.message);
    return {};  // Returns empty object, hiding the failure
  }
}
```

### Good Code (Fix)
```javascript
async function saveUser(user) {
  try {
    await db.insert('users', user);
    await sendWelcomeEmail(user.email);
    await notifyAdmin(user);
  } catch (error) {
    logger.error('Failed to save user', { userId: user.id, error });
    throw new UserCreationError('Could not create user', { cause: error });
  }
  return { success: true };
}

function parseConfig(path) {
  try {
    return JSON.parse(fs.readFileSync(path, 'utf-8'));
  } catch (e) {
    logger.error('Config parse failed', { path, error: e.message });
    throw new ConfigError(`Invalid config at ${path}`, { cause: e });
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `catch_clause`, `statement_block`, `call_expression`, `member_expression`
- **Detection approach**: Find `catch_clause` bodies that contain a `call_expression` targeting `console.log`, `console.error`, or `console.warn` but do not contain a `throw_statement` or `return` statement that propagates the error. Check that no `throw_statement` exists as a descendant of the catch body.
- **S-expression query sketch**:
```scheme
(catch_clause
  body: (statement_block
    (expression_statement
      (call_expression
        function: (member_expression
          object: (identifier) @console_obj
          property: (property_identifier) @log_method)))))
```

### Pipeline Mapping
- **Pipeline name**: `unhandled_promise`
- **Pattern name**: `swallowed_error`
- **Severity**: warning
- **Confidence**: medium
