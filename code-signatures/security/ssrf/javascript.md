# SSRF -- JavaScript

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In JavaScript/TypeScript (Node.js) applications, this commonly manifests through `fetch()`, `axios`, `http.request()`, or similar HTTP clients receiving unsanitized URLs from request parameters, headers, or body fields. Attackers exploit SSRF to reach internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network firewalls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: node-fetch, axios, undici, http/https (Node.js built-in), got, superagent, express (redirect handling)

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to an HTTP client function such as `fetch()`, `axios.get()`, or `http.request()` without validating the URL scheme, hostname, or IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```typescript
import express from 'express';
import axios from 'axios';

const app = express();

app.get('/proxy', async (req, res) => {
  const targetUrl = req.query.url as string;
  // User-controlled URL passed directly to HTTP client
  const response = await axios.get(targetUrl);
  res.json(response.data);
});

app.get('/fetch-page', async (req, res) => {
  const url = req.query.url as string;
  // fetch() with user-controlled URL
  const response = await fetch(url);
  const body = await response.text();
  res.send(body);
});
```

### Good Code (Fix)
```typescript
import express from 'express';
import axios from 'axios';
import { URL } from 'url';
import { isIP } from 'net';
import dns from 'dns/promises';

const ALLOWED_HOSTS = new Set(['api.example.com', 'cdn.example.com']);
const BLOCKED_RANGES = ['127.', '10.', '172.16.', '192.168.', '169.254.', '0.'];

async function validateUrl(input: string): Promise<URL> {
  const parsed = new URL(input);
  if (!['https:', 'http:'].includes(parsed.protocol)) {
    throw new Error('Only HTTP(S) URLs are allowed');
  }
  if (!ALLOWED_HOSTS.has(parsed.hostname)) {
    throw new Error('Host not in allowlist');
  }
  // Resolve DNS to prevent DNS rebinding to internal IPs
  const addresses = await dns.resolve4(parsed.hostname);
  for (const addr of addresses) {
    if (BLOCKED_RANGES.some(prefix => addr.startsWith(prefix))) {
      throw new Error('Resolved to blocked IP range');
    }
  }
  return parsed;
}

const app = express();

app.get('/proxy', async (req, res) => {
  try {
    const validated = await validateUrl(req.query.url as string);
    const response = await axios.get(validated.toString());
    res.json(response.data);
  } catch (e) {
    res.status(400).json({ error: 'Invalid URL' });
  }
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `await_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the callee is `fetch`, `axios.get`, `axios.post`, `axios.request`, `http.request`, or `http.get`, and the first argument is an `identifier` or `member_expression` referencing a variable that originates from user input (e.g., `req.query`, `req.params`, `req.body`). Flag cases where no URL validation function is called before the HTTP request.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (arguments
    (identifier) @url_arg))

(call_expression
  function: (member_expression
    object: (identifier) @http_client
    property: (property_identifier) @method)
  arguments: (arguments
    (identifier) @url_arg))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `user_controlled_url_http_request`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Open Redirect via User-Controlled Redirect Target

### Description
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin path or belongs to an allowlisted set of domains. While open redirects are often classified as a separate vulnerability, they become an SSRF vector when server-side redirect-following HTTP clients (e.g., `axios`, `fetch` with `redirect: 'follow'`) chase the redirect to an attacker-controlled destination, or when the redirect itself is used in phishing chains.

### Bad Code (Anti-pattern)
```typescript
import express from 'express';

const app = express();

app.get('/redirect', (req, res) => {
  const target = req.query.url as string;
  // User-controlled redirect target -- attacker can redirect to any URL
  res.redirect(target);
});

app.get('/login-callback', (req, res) => {
  const returnTo = req.query.returnTo as string;
  // Redirect after login with no validation
  res.redirect(302, returnTo);
});
```

### Good Code (Fix)
```typescript
import express from 'express';
import { URL } from 'url';

const ALLOWED_REDIRECT_DOMAINS = new Set(['example.com', 'app.example.com']);

function isSafeRedirect(target: string, requestHost: string): boolean {
  // Allow relative paths (same-origin)
  if (target.startsWith('/') && !target.startsWith('//')) {
    return true;
  }
  try {
    const parsed = new URL(target);
    return ALLOWED_REDIRECT_DOMAINS.has(parsed.hostname);
  } catch {
    return false;
  }
}

const app = express();

app.get('/redirect', (req, res) => {
  const target = req.query.url as string;
  if (!isSafeRedirect(target, req.hostname)) {
    return res.status(400).send('Invalid redirect target');
  }
  res.redirect(target);
});

app.get('/login-callback', (req, res) => {
  const returnTo = req.query.returnTo as string;
  if (!isSafeRedirect(returnTo, req.hostname)) {
    return res.redirect('/');
  }
  res.redirect(302, returnTo);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the callee is `res.redirect` and the argument is an `identifier` or `member_expression` tracing back to user input (`req.query`, `req.params`, `req.body`). Flag when no URL validation or allowlist check precedes the redirect call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @res_obj
    property: (property_identifier) @redirect_method)
  arguments: (arguments
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
