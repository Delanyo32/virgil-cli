# Resource Exhaustion -- JavaScript

## Overview
Resource exhaustion vulnerabilities in JavaScript arise when an application allows unbounded consumption of CPU, memory, or other system resources based on user-controlled input. The most critical vectors are Regular Expression Denial of Service (ReDoS) -- where crafted input triggers catastrophic backtracking in regex engines -- and unbounded request body parsing where HTTP servers accept arbitrarily large payloads into memory.

## Why It's a Security Concern
ReDoS can render a Node.js server completely unresponsive by locking the single-threaded event loop on a single malicious request. Unbounded body parsing allows attackers to exhaust server memory with oversized requests, causing out-of-memory crashes and denial of service for all users. Both attacks are cheap for adversaries (a single HTTP request) but devastating for availability.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Express, Koa, body-parser, Fastify, native RegExp

---

## Pattern 1: ReDoS -- Catastrophic Backtracking Regex on User Input

### Description
Using a regular expression with nested quantifiers, overlapping alternations, or other backtracking-prone constructs to validate or process user-supplied input. The JavaScript regex engine uses backtracking, and patterns like `(a+)+`, `(a|a)*`, or `(\w+\.)+\w+` can exhibit exponential time complexity on crafted input.

### Bad Code (Anti-pattern)
```typescript
import { Request, Response } from 'express';

function validateEmail(req: Request, res: Response) {
  const email = req.body.email;
  // Nested quantifiers cause catastrophic backtracking
  const emailRegex = /^([a-zA-Z0-9_\.\-]+)+@([a-zA-Z0-9\-]+\.)+[a-zA-Z]{2,}$/;
  if (emailRegex.test(email)) {
    res.json({ valid: true });
  } else {
    res.status(400).json({ valid: false });
  }
}

function matchTags(userInput: string): string[] {
  // Overlapping alternations with repetition
  const tagRegex = /(<[^>]*>)*.*(<\/[^>]*>)*/g;
  return userInput.match(tagRegex) || [];
}
```

### Good Code (Fix)
```typescript
import { Request, Response } from 'express';

function validateEmail(req: Request, res: Response) {
  const email = req.body.email;
  // Simple, non-backtracking pattern with length limit
  if (email.length > 254) {
    return res.status(400).json({ valid: false });
  }
  const emailRegex = /^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$/;
  if (emailRegex.test(email)) {
    res.json({ valid: true });
  } else {
    res.status(400).json({ valid: false });
  }
}

function matchTags(userInput: string): string[] {
  // Use a proper HTML parser instead of regex
  const { JSDOM } = require('jsdom');
  const dom = new JSDOM(userInput);
  return Array.from(dom.window.document.querySelectorAll('*'))
    .map((el: Element) => el.tagName.toLowerCase());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `regex`, `regex_pattern`, `call_expression`, `new_expression`
- **Detection approach**: Find `regex` literal nodes or `new RegExp()` constructor calls. Analyze the pattern string for nested quantifiers (`(x+)+`, `(x*)*`, `(x+)*`), overlapping character class alternations, or other constructs known to cause super-linear backtracking. Flag when the regex is applied to user-controlled input (function parameters, request properties).
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (regex) @pattern
    property: (property_identifier) @method)
  arguments: (arguments
    (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `redos_resource_exhaustion`
- **Pattern name**: `regex_catastrophic_backtracking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Unbounded Request Body Parsing -- No Body Size Limit

### Description
Configuring Express, Koa, or similar frameworks to parse request bodies (JSON, URL-encoded, raw) without specifying a maximum size limit. The default limits in some middleware are generous (e.g., `body-parser` defaults to 100KB) but explicitly disabling or raising them, or using custom parsing without limits, allows attackers to submit arbitrarily large payloads.

### Bad Code (Anti-pattern)
```typescript
import express from 'express';

const app = express();

// No size limit -- accepts arbitrarily large JSON bodies
app.use(express.json({ limit: Infinity }));

// Reading raw body without any size constraint
app.post('/upload', (req, res) => {
  const chunks: Buffer[] = [];
  req.on('data', (chunk: Buffer) => {
    chunks.push(chunk); // No size tracking or limit
  });
  req.on('end', () => {
    const body = Buffer.concat(chunks);
    res.json({ size: body.length });
  });
});
```

### Good Code (Fix)
```typescript
import express from 'express';

const app = express();

// Enforce a reasonable size limit
app.use(express.json({ limit: '1mb' }));

// Track accumulated size and abort if too large
app.post('/upload', (req, res) => {
  const MAX_SIZE = 10 * 1024 * 1024; // 10 MB
  const chunks: Buffer[] = [];
  let totalSize = 0;

  req.on('data', (chunk: Buffer) => {
    totalSize += chunk.length;
    if (totalSize > MAX_SIZE) {
      req.destroy();
      return res.status(413).json({ error: 'Payload too large' });
    }
    chunks.push(chunk);
  });
  req.on('end', () => {
    const body = Buffer.concat(chunks);
    res.json({ size: body.length });
  });
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `object`, `property`, `identifier`
- **Detection approach**: Find `call_expression` nodes invoking `express.json()`, `express.urlencoded()`, `express.raw()`, or `bodyParser.json()` with an options object where `limit` is set to `Infinity`, a very large number, or is absent. Also find raw `req.on('data', ...)` handlers that accumulate chunks without checking total size.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @framework
    property: (property_identifier) @method)
  arguments: (arguments
    (object
      (pair
        key: (property_identifier) @key
        value: (_) @value))))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_body_parsing`
- **Severity**: warning
- **Confidence**: medium
