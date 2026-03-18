# Cryptography Weakness -- PHP

## Overview
Cryptographic weaknesses in PHP occur when developers use non-cryptographic random functions like `rand()`, `mt_rand()`, or `array_rand()` for security-sensitive operations, or use `md5()` / `sha1()` for password hashing instead of PHP's built-in `password_hash()`. PHP has historically had a weak security posture around randomness and hashing, but modern PHP (7.0+) provides robust alternatives that many codebases have not yet adopted.

## Why It's a Security Concern
`rand()` uses a platform-dependent PRNG, and `mt_rand()` uses Mersenne Twister -- both are deterministic and can have their internal state reconstructed from observed outputs (mt_rand requires only 624 outputs). Tokens, CSRF nonces, and password reset links generated with these functions are predictable. Using `md5()` or `sha1()` for password hashing provides no salt, no key stretching, and can be brute-forced at billions of hashes per second. PHP's `password_hash()` with `PASSWORD_BCRYPT` or `PASSWORD_ARGON2ID` is the correct approach.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: Core PHP (rand, mt_rand, random_int, random_bytes), password_hash, md5, sha1, hash, Laravel Hash, Symfony PasswordHasher

---

## Pattern 1: Using rand()/mt_rand() Instead of random_int()/random_bytes()

### Description
Using `rand()`, `mt_rand()`, `array_rand()`, or `shuffle()` to generate security-sensitive values such as tokens, CSRF nonces, password reset codes, OTP codes, or encryption keys. These functions are not cryptographically secure. PHP 7+ provides `random_int()` and `random_bytes()` which use the operating system's CSPRNG.

### Bad Code (Anti-pattern)
```php
function generateToken(int $length = 32): string {
    $chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    $token = '';
    for ($i = 0; $i < $length; $i++) {
        $token .= $chars[mt_rand(0, strlen($chars) - 1)];
    }
    return $token;
}

function generateCSRFToken(): string {
    return md5(rand());
}
```

### Good Code (Fix)
```php
function generateToken(int $length = 32): string {
    return bin2hex(random_bytes($length));
}

function generateCSRFToken(): string {
    return bin2hex(random_bytes(32));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `arguments`
- **Detection approach**: Find `function_call_expression` nodes where the function name is `rand`, `mt_rand`, `mt_srand`, `srand`, or `array_rand`. These functions are inherently non-cryptographic. Context clues such as assignment to variables named `$token`, `$csrf`, `$nonce`, `$key`, `$secret`, or `$otp` increase confidence.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func
  (#match? @func "^(rand|mt_rand|mt_srand|srand|array_rand)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `rand_mt_rand_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using md5()/sha1() for Password Hashing Instead of password_hash()

### Description
Using `md5($password)` or `sha1($password)` to hash passwords before storage. These are fast general-purpose hash functions with no salting or key stretching. PHP provides `password_hash()` (since PHP 5.5) which implements bcrypt by default and argon2 in PHP 7.2+, with automatic salting and configurable cost factors. There is no valid reason to use md5/sha1 for password storage in modern PHP.

### Bad Code (Anti-pattern)
```php
function registerUser(string $username, string $password): void {
    $hashedPassword = md5($password);
    $db->query("INSERT INTO users (username, password) VALUES (?, ?)", [$username, $hashedPassword]);
}

function verifyLogin(string $password, string $storedHash): bool {
    return sha1($password) === $storedHash;
}
```

### Good Code (Fix)
```php
function registerUser(string $username, string $password): void {
    $hashedPassword = password_hash($password, PASSWORD_BCRYPT, ['cost' => 12]);
    $db->query("INSERT INTO users (username, password) VALUES (?, ?)", [$username, $hashedPassword]);
}

function verifyLogin(string $password, string $storedHash): bool {
    return password_verify($password, $storedHash);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `arguments`, `variable_name`
- **Detection approach**: Find `function_call_expression` nodes calling `md5` or `sha1` where the argument is a variable (not a string literal -- string literals may be checksumming static data). Context of password handling (variable names containing `password`, `passwd`, `pwd`, surrounding function names like `register`, `login`, `verify`, `authenticate`) confirms the anti-pattern.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func
  arguments: (arguments
    (variable_name) @input)
  (#match? @func "^(md5|sha1)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_password_hash`
- **Severity**: error
- **Confidence**: high
