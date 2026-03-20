# Memory Safety -- JavaScript

## Overview
JavaScript is a memory-safe language with garbage collection, so traditional memory corruption vulnerabilities like buffer overflows and use-after-free are not possible in pure JS. However, prototype pollution is a memory-safety-adjacent vulnerability where an attacker modifies `Object.prototype` through recursive merge/extend functions that process user-controlled input, effectively corrupting the shared memory layout of all objects in the runtime.

## Why It's a Security Concern
Prototype pollution allows an attacker to inject properties (e.g., `__proto__.isAdmin = true`) into the base object prototype. Every object in the application then inherits these poisoned properties, leading to authentication bypasses, remote code execution (via polluted properties consumed by template engines or child_process), and denial of service.

## Applicability
- **Relevance**: medium
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: lodash (_.merge, _.defaultsDeep), jQuery ($.extend), hoek, deep-extend, custom recursive merge utilities

---

## Pattern 1: Prototype Pollution via Recursive Merge/Extend

### Description
Recursive object merge or extend functions that copy properties from a user-controlled source object into a target without filtering `__proto__`, `constructor`, or `prototype` keys. This allows an attacker to inject properties into `Object.prototype` by submitting JSON like `{"__proto__": {"isAdmin": true}}`.

### Bad Code (Anti-pattern)
```javascript
function deepMerge(target, source) {
    for (const key in source) {
        if (typeof source[key] === 'object' && source[key] !== null) {
            if (!target[key]) target[key] = {};
            deepMerge(target[key], source[key]);
        } else {
            target[key] = source[key];
        }
    }
    return target;
}

// Attacker sends: {"__proto__": {"isAdmin": true}}
const userInput = JSON.parse(req.body);
deepMerge(config, userInput);
// Now ({}).isAdmin === true for ALL objects
```

### Good Code (Fix)
```javascript
function deepMerge(target, source) {
    for (const key in source) {
        if (key === '__proto__' || key === 'constructor' || key === 'prototype') {
            continue;
        }
        if (typeof source[key] === 'object' && source[key] !== null) {
            if (!target[key]) target[key] = {};
            deepMerge(target[key], source[key]);
        } else {
            target[key] = source[key];
        }
    }
    return target;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `for_in_statement`, `subscript_expression`, `member_expression`
- **Detection approach**: Find functions that iterate over object keys (`for...in`, `Object.keys`) and perform recursive assignment via bracket notation (`target[key]`) without checking for `__proto__`, `constructor`, or `prototype`. Flag recursive merge/extend functions that accept external input and lack prototype key filtering.
- **S-expression query sketch**:
```scheme
(for_in_statement
  left: (identifier) @key_var
  right: (_) @source
  body: (statement_block
    (if_statement
      consequence: (statement_block
        (expression_statement
          (call_expression
            function: (identifier) @func_name))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_safety`
- **Pattern name**: `prototype_pollution`
- **Severity**: error
- **Confidence**: medium
