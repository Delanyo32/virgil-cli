# Memory Leak Indicators -- JavaScript

## Overview
Memory leaks in JavaScript occur when references to objects are retained beyond their useful lifetime, preventing garbage collection. Common sources include event listeners added without removal, uncleared intervals, and unbounded collection growth.

## Why It's a Scalability Concern
JavaScript's garbage collector cannot reclaim memory held by lingering references. In long-running Node.js servers or single-page applications, leaked memory accumulates over hours/days, increasing heap usage until the process is killed by OOM or GC pauses become unacceptable. Each leaked listener or cached object compounds under concurrent usage.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: DOM API, Node.js EventEmitter, Map/Set/WeakMap
- **Existing pipeline**: `event_listener_leak.rs` in `src/audit/pipelines/javascript/` — extends with additional patterns

---

## Pattern 1: addEventListener Without removeEventListener

### Description
Calling `addEventListener()` without a corresponding `removeEventListener()` in the same scope or cleanup path, causing listeners to accumulate.

### Bad Code (Anti-pattern)
```javascript
function initComponent(element) {
  element.addEventListener('click', handleClick);
  element.addEventListener('scroll', handleScroll);
  // no cleanup — listeners persist after component unmount
}
```

### Good Code (Fix)
```javascript
function initComponent(element) {
  element.addEventListener('click', handleClick);
  element.addEventListener('scroll', handleScroll);
  return () => {
    element.removeEventListener('click', handleClick);
    element.removeEventListener('scroll', handleScroll);
  };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`
- **Detection approach**: Find `call_expression` with `member_expression` where property is `addEventListener`. Then search the same scope (function body or block) for a corresponding `removeEventListener` with the same event name. Flag if no matching removal exists.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    property: (property_identifier) @method)
  (#eq? @method "addEventListener"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `event_listener_no_cleanup`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: setInterval Without clearInterval

### Description
Calling `setInterval()` without storing the return value for later `clearInterval()`, making it impossible to stop the interval.

### Bad Code (Anti-pattern)
```javascript
function startPolling(url) {
  setInterval(async () => {
    const data = await fetch(url);
    updateUI(await data.json());
  }, 5000);
}
```

### Good Code (Fix)
```javascript
function startPolling(url) {
  const intervalId = setInterval(async () => {
    const data = await fetch(url);
    updateUI(await data.json());
  }, 5000);
  return () => clearInterval(intervalId);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `expression_statement`
- **Detection approach**: Find `call_expression` calling `setInterval` that is inside an `expression_statement` (not assigned to a variable). If the return value is not captured, the interval cannot be cleared.
- **S-expression query sketch**:
```scheme
(expression_statement
  (call_expression
    function: (identifier) @func_name
    (#eq? @func_name "setInterval")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `interval_no_clear`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Unbounded Map/Set/Object Growth

### Description
Adding entries to a `Map`, `Set`, or object via property assignment inside a loop or recurring function without any eviction, deletion, or size limit.

### Bad Code (Anti-pattern)
```typescript
const cache = new Map<string, any>();

function processRequest(req: Request) {
  const key = req.url + req.headers['authorization'];
  if (!cache.has(key)) {
    cache.set(key, computeExpensiveResult(req));
  }
  return cache.get(key);
}
```

### Good Code (Fix)
```typescript
const cache = new Map<string, { value: any; timestamp: number }>();
const MAX_CACHE_SIZE = 10000;

function processRequest(req: Request) {
  const key = req.url + req.headers['authorization'];
  if (!cache.has(key)) {
    if (cache.size >= MAX_CACHE_SIZE) {
      const oldest = cache.keys().next().value;
      cache.delete(oldest);
    }
    cache.set(key, { value: computeExpensiveResult(req), timestamp: Date.now() });
  }
  return cache.get(key).value;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`
- **Detection approach**: Find `call_expression` calling `.set()` on a `Map` or `.add()` on a `Set` (or property assignment on an object) where no `.delete()`, `.clear()`, or size check exists in the same scope or module.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @collection
    property: (property_identifier) @method)
  (#eq? @method "set"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `unbounded_collection_growth`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: EventEmitter.on() Without .off()

### Description
Adding Node.js `EventEmitter` listeners with `.on()` or `.addListener()` without corresponding `.off()` or `.removeListener()`, especially in code that runs repeatedly.

### Bad Code (Anti-pattern)
```javascript
function handleConnection(socket) {
  process.on('SIGTERM', () => {
    socket.end();
  });
}
```

### Good Code (Fix)
```javascript
function handleConnection(socket) {
  const onSigterm = () => {
    socket.end();
  };
  process.on('SIGTERM', onSigterm);
  socket.on('close', () => {
    process.off('SIGTERM', onSigterm);
  });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`
- **Detection approach**: Find `call_expression` calling `.on()` or `.addListener()` on an object, then check the same scope for `.off()`, `.removeListener()`, or `.removeAllListeners()` on the same object. Flag if no removal exists.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @emitter
    property: (property_identifier) @method)
  (#match? @method "^(on|addListener)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `emitter_listener_no_cleanup`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 5: Global Cache Map Without Eviction

### Description
A module-level `Map` or object used as a cache with `.set()` calls but no `.delete()`, `.clear()`, or TTL mechanism anywhere in the module.

### Bad Code (Anti-pattern)
```javascript
const userCache = new Map();

export function getUser(id) {
  if (!userCache.has(id)) {
    userCache.set(id, fetchUserFromDb(id));
  }
  return userCache.get(id);
}
```

### Good Code (Fix)
```javascript
import { LRUCache } from 'lru-cache';

const userCache = new LRUCache({ max: 500, ttl: 1000 * 60 * 5 });

export function getUser(id) {
  if (!userCache.has(id)) {
    userCache.set(id, fetchUserFromDb(id));
  }
  return userCache.get(id);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `lexical_declaration`, `new_expression`, `identifier`, `call_expression`
- **Detection approach**: Find module-level (top-level `lexical_declaration`) `Map` or `Set` construction via `new Map()` / `new Set()`. Then check the entire module for `.set()` / `.add()` calls on that variable WITHOUT any `.delete()` or `.clear()` calls on the same variable.
- **S-expression query sketch**:
```scheme
(program
  (lexical_declaration
    (variable_declarator
      name: (identifier) @cache_name
      value: (new_expression
        constructor: (identifier) @type
        (#eq? @type "Map")))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `global_cache_no_eviction`
- **Severity**: warning
- **Confidence**: medium
