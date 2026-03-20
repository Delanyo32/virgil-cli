# Cryptography Weakness -- JavaScript

## Overview
Cryptographic weaknesses in JavaScript occur when developers use non-cryptographic randomness sources like `Math.random()` for security-sensitive operations, or rely on broken/weak hash algorithms (MD5, SHA-1) for password storage. JavaScript's `Math.random()` is a PRNG seeded from a predictable source and produces output that can be predicted or brute-forced, while MD5 and SHA-1 are vulnerable to collision attacks and offer no resistance to brute-force password cracking.

## Why It's a Security Concern
Using `Math.random()` for generating tokens, session IDs, passwords, or cryptographic keys produces predictable values that attackers can guess or reconstruct. Modern browsers expose the internal PRNG state, making `Math.random()` output fully deterministic once a few values are observed. Storing passwords hashed with MD5 or SHA-1 allows attackers with a stolen database to crack passwords in seconds using rainbow tables or GPU-accelerated brute force. Both patterns lead to authentication bypass, account takeover, and data breach.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Node.js crypto, Web Crypto API, bcrypt, scrypt, argon2, createHash, createHmac

---

## Pattern 1: Math.random() Used for Security-Sensitive Operations

### Description
Using `Math.random()` to generate tokens, passwords, session identifiers, OTP codes, or cryptographic keys. `Math.random()` is not a cryptographically secure PRNG -- its output is predictable and must never be used where unpredictability is a security requirement. The correct alternative is `crypto.randomBytes()` in Node.js or `crypto.getRandomValues()` in browsers.

### Bad Code (Anti-pattern)
```typescript
function generateToken(length: number): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  let token = '';
  for (let i = 0; i < length; i++) {
    token += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return token;
}

function generateSessionId(): string {
  return Math.random().toString(36).substring(2);
}
```

### Good Code (Fix)
```typescript
import crypto from 'crypto';

function generateToken(length: number): string {
  return crypto.randomBytes(length).toString('hex');
}

function generateSessionId(): string {
  return crypto.randomUUID();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`
- **Detection approach**: Find `call_expression` nodes where the callee is `Math.random` (a `member_expression` with object `Math` and property `random`). Flag all occurrences, as `Math.random()` should never be used in security-sensitive contexts. Context clues such as assignment to variables named `token`, `secret`, `key`, `session`, or `password` increase confidence.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  (#eq? @obj "Math")
  (#eq? @method "random"))
```

### Pipeline Mapping
- **Pipeline name**: `timing_weak_crypto`
- **Pattern name**: `math_random_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Weak Hashing (MD5/SHA-1) for Password Storage

### Description
Using `crypto.createHash('md5')` or `crypto.createHash('sha1')` to hash passwords before storing them. MD5 and SHA-1 are fast general-purpose hash functions that are trivially brute-forced on modern hardware -- billions of hashes per second on a single GPU. Passwords must be hashed with a purpose-built password hashing function (bcrypt, scrypt, or argon2) that incorporates salting, key stretching, and tunable cost parameters.

### Bad Code (Anti-pattern)
```typescript
import crypto from 'crypto';

function hashPassword(password: string): string {
  return crypto.createHash('md5').update(password).digest('hex');
}

function verifyPassword(password: string, storedHash: string): boolean {
  const hash = crypto.createHash('sha1').update(password).digest('hex');
  return hash === storedHash;
}
```

### Good Code (Fix)
```typescript
import bcrypt from 'bcrypt';

async function hashPassword(password: string): Promise<string> {
  const saltRounds = 12;
  return bcrypt.hash(password, saltRounds);
}

async function verifyPassword(password: string, storedHash: string): Promise<boolean> {
  return bcrypt.compare(password, storedHash);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `string`, `arguments`
- **Detection approach**: Find `call_expression` nodes where the callee is `crypto.createHash` (or a destructured `createHash`) and the first argument is the string literal `'md5'` or `'sha1'`. Context analysis of surrounding code (variable names containing `password`, `passwd`, `pwd`) increases confidence that this is password hashing rather than checksum usage.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @algo)
  (#eq? @method "createHash")
  (#match? @algo "md5|sha1"))
```

### Pipeline Mapping
- **Pipeline name**: `timing_weak_crypto`
- **Pattern name**: `weak_password_hash`
- **Severity**: error
- **Confidence**: high
