# Insecure Deserialization -- JavaScript

## Overview
Insecure deserialization in JavaScript occurs when untrusted data is parsed or evaluated without proper validation, allowing attackers to inject malicious payloads. Common vectors include `JSON.parse()` without schema validation and `eval()`/`Function()` used to reconstruct data from strings.

## Why It's a Security Concern
Deserializing untrusted input without validation can lead to prototype pollution, injection of unexpected object shapes, or outright remote code execution when `eval` or `Function` constructors are used. Attackers craft payloads that exploit the deserialization path to execute arbitrary code, escalate privileges, or corrupt application state.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Node.js core, Express, any code accepting JSON from external sources

---

## Pattern 1: JSON.parse of Untrusted Input Without Schema Validation

### Description
Calling `JSON.parse()` on user-controlled input (request bodies, query parameters, WebSocket messages, file contents) without validating the resulting object against a schema. The parsed object may contain unexpected fields, `__proto__` pollution payloads, or values of unexpected types.

### Bad Code (Anti-pattern)
```typescript
app.post('/api/config', (req, res) => {
  const config = JSON.parse(req.body.rawConfig);
  applyConfig(config); // No validation — attacker controls object shape
});
```

### Good Code (Fix)
```typescript
import { z } from 'zod';

const ConfigSchema = z.object({
  theme: z.string(),
  pageSize: z.number().int().positive(),
  features: z.array(z.string()),
});

app.post('/api/config', (req, res) => {
  const parsed = JSON.parse(req.body.rawConfig);
  const config = ConfigSchema.parse(parsed); // Validates shape and types
  applyConfig(config);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the callee is `JSON.parse` (a `member_expression` with object `JSON` and property `parse`). Flag when the result is used directly without a subsequent validation call (e.g., no `.parse()`, `.validate()`, `.safeParse()` in the same scope).
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  arguments: (arguments (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `json_parse_no_validation`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: eval/Function Constructor to Deserialize Data

### Description
Using `eval()`, `new Function()`, or `Function()` to reconstruct objects or execute serialized code strings. This grants full code execution to the attacker if the input is untrusted.

### Bad Code (Anti-pattern)
```javascript
function deserialize(serialized) {
  return eval('(' + serialized + ')'); // RCE: attacker sends arbitrary JS
}

// Or via Function constructor:
function deserialize(serialized) {
  const fn = new Function('return ' + serialized);
  return fn();
}
```

### Good Code (Fix)
```javascript
function deserialize(serialized) {
  const data = JSON.parse(serialized); // Safe parsing, no code execution
  return validateSchema(data);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `new_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the callee is `eval` or `new_expression` nodes constructing `Function`. Flag when the argument includes a variable or concatenation (indicating dynamic input rather than a static string).
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (arguments (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `eval_deserialize`
- **Severity**: error
- **Confidence**: high
