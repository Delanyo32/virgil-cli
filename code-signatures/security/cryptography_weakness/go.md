# Cryptography Weakness -- Go

## Overview
Cryptographic weaknesses in Go arise when developers use `math/rand` instead of `crypto/rand` for generating security-sensitive values, or rely on weak hash algorithms (MD5, SHA-1) for security purposes. Go's standard library clearly separates these concerns into distinct packages, but the similar API surfaces lead to frequent misuse, especially by developers coming from languages without this separation.

## Why It's a Security Concern
`math/rand` uses a deterministic PRNG that, prior to Go 1.20, was seeded with a fixed value (0) by default -- making all output entirely predictable. Even with proper seeding, `math/rand` is not cryptographically secure and its internal state can be reconstructed from observed outputs. Using `crypto/md5` or `crypto/sha1` for password hashing, token generation, or integrity verification exposes applications to collision attacks and brute-force cracking at rates exceeding billions of hashes per second on commodity hardware.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: math/rand, crypto/rand, crypto/md5, crypto/sha1, crypto/sha256, golang.org/x/crypto/bcrypt, golang.org/x/crypto/argon2, golang.org/x/crypto/scrypt

---

## Pattern 1: Using math/rand Instead of crypto/rand

### Description
Using `math/rand.Intn()`, `math/rand.Int()`, `math/rand.Read()`, or any `math/rand` function to generate tokens, session IDs, passwords, encryption keys, or nonces. The `math/rand` package is designed for simulations and non-security uses. For cryptographic randomness, `crypto/rand.Read()` or `crypto/rand.Int()` must be used.

### Bad Code (Anti-pattern)
```go
import (
    "math/rand"
    "time"
)

func generateToken(length int) string {
    rand.Seed(time.Now().UnixNano())
    const charset = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
    token := make([]byte, length)
    for i := range token {
        token[i] = charset[rand.Intn(len(charset))]
    }
    return string(token)
}
```

### Good Code (Fix)
```go
import (
    "crypto/rand"
    "encoding/hex"
)

func generateToken(length int) (string, error) {
    bytes := make([]byte, length)
    if _, err := rand.Read(bytes); err != nil {
        return "", err
    }
    return hex.EncodeToString(bytes), nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `import_declaration`
- **Detection approach**: Find `import_declaration` nodes that import `math/rand`. Then find `call_expression` nodes using functions from this package -- `rand.Intn`, `rand.Int`, `rand.Read`, `rand.Seed`, `rand.Float64`, etc. The presence of a `math/rand` import (as opposed to `crypto/rand`) is the primary signal. Context of token/key generation increases confidence.
- **S-expression query sketch**:
```scheme
(import_spec
  path: (interpreted_string_literal) @import_path
  (#eq? @import_path "\"math/rand\""))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `math_rand_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using MD5/SHA-1 for Security Purposes

### Description
Using `crypto/md5.Sum()`, `crypto/md5.New()`, `crypto/sha1.Sum()`, or `crypto/sha1.New()` for password hashing, message authentication, digital signatures, or any context where collision resistance matters. MD5 and SHA-1 are cryptographically broken for these purposes. Use `crypto/sha256` or stronger for integrity, and `golang.org/x/crypto/bcrypt` or `argon2` for passwords.

### Bad Code (Anti-pattern)
```go
import (
    "crypto/md5"
    "crypto/sha1"
    "fmt"
)

func hashPassword(password string) string {
    hash := md5.Sum([]byte(password))
    return fmt.Sprintf("%x", hash)
}

func computeMAC(data []byte) []byte {
    h := sha1.New()
    h.Write(data)
    return h.Sum(nil)
}
```

### Good Code (Fix)
```go
import (
    "golang.org/x/crypto/bcrypt"
    "crypto/sha256"
)

func hashPassword(password string) (string, error) {
    hash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
    if err != nil {
        return "", err
    }
    return string(hash), nil
}

func computeMAC(data []byte) []byte {
    h := sha256.New()
    h.Write(data)
    return h.Sum(nil)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `import_declaration`, `identifier`
- **Detection approach**: Find `import_declaration` nodes importing `crypto/md5` or `crypto/sha1`. Then find `call_expression` nodes using `md5.Sum`, `md5.New`, `sha1.Sum`, or `sha1.New`. The presence of the import combined with usage in password or authentication contexts (function names, variable names) confirms the vulnerability.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @func)
  (#match? @pkg "^(md5|sha1)$")
  (#match? @func "^(Sum|New)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_hash_security`
- **Severity**: error
- **Confidence**: high
