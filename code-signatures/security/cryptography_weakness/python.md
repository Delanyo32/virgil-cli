# Cryptography Weakness -- Python

## Overview
Cryptographic weaknesses in Python occur when developers use the `random` module (which implements a Mersenne Twister PRNG) for security-sensitive operations, or rely on fast general-purpose hash functions like MD5 or SHA-1 for password storage. Python's `random` module explicitly states in its documentation that it is "completely unsuitable for cryptographic purposes," yet it is frequently misused for generating tokens, passwords, and secrets.

## Why It's a Security Concern
The `random` module's Mersenne Twister has a predictable internal state that can be fully reconstructed from 624 consecutive outputs. An attacker observing a small number of generated values can predict all future and past outputs, compromising tokens, session IDs, or OTPs. Storing passwords hashed with `hashlib.md5()` or `hashlib.sha1()` allows offline brute-force attacks at billions of attempts per second -- a leaked database of MD5-hashed passwords can be fully cracked in hours.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: random, secrets, hashlib, bcrypt, argon2-cffi, passlib, Django auth, Flask-Bcrypt

---

## Pattern 1: Using random Module Instead of secrets for Security Tokens

### Description
Using `random.choice()`, `random.randint()`, `random.randrange()`, `random.getrandbits()`, or `random.sample()` to generate tokens, passwords, API keys, or other security-sensitive values. The `random` module uses a deterministic PRNG (Mersenne Twister) that is not suitable for cryptographic purposes. The `secrets` module (Python 3.6+) provides cryptographically strong random values.

### Bad Code (Anti-pattern)
```python
import random
import string

def generate_api_key(length: int = 32) -> str:
    chars = string.ascii_letters + string.digits
    return ''.join(random.choice(chars) for _ in range(length))

def generate_otp() -> str:
    return str(random.randint(100000, 999999))
```

### Good Code (Fix)
```python
import secrets
import string

def generate_api_key(length: int = 32) -> str:
    return secrets.token_urlsafe(length)

def generate_otp() -> str:
    return str(secrets.randbelow(900000) + 100000)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`, `argument_list`
- **Detection approach**: Find `call` nodes where the function is an `attribute` on the `random` module -- specifically `random.choice`, `random.randint`, `random.randrange`, `random.getrandbits`, `random.sample`, or `random.random`. Also detect direct calls to these after `from random import choice, randint`. Variable names or surrounding context containing `token`, `key`, `secret`, `password`, `otp` increase confidence.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  (#eq? @module "random")
  (#match? @method "^(choice|randint|randrange|getrandbits|sample|random)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `random_module_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using hashlib MD5/SHA-1 for Password Hashing

### Description
Using `hashlib.md5()` or `hashlib.sha1()` to hash passwords before storing them. These are fast cryptographic hash functions designed for integrity checking, not password storage. They offer no protection against brute-force or rainbow table attacks. Passwords must be hashed with dedicated password hashing functions like `bcrypt`, `argon2`, or `scrypt` that incorporate salting and key stretching.

### Bad Code (Anti-pattern)
```python
import hashlib

def hash_password(password: str) -> str:
    return hashlib.md5(password.encode()).hexdigest()

def verify_password(password: str, stored_hash: str) -> bool:
    return hashlib.sha1(password.encode()).hexdigest() == stored_hash
```

### Good Code (Fix)
```python
import bcrypt

def hash_password(password: str) -> bytes:
    salt = bcrypt.gensalt(rounds=12)
    return bcrypt.hashpw(password.encode(), salt)

def verify_password(password: str, stored_hash: bytes) -> bool:
    return bcrypt.checkpw(password.encode(), stored_hash)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `identifier`, `argument_list`
- **Detection approach**: Find `call` nodes where the function is `hashlib.md5` or `hashlib.sha1` (an `attribute` on `hashlib`). Also detect `hashlib.new('md5')` or `hashlib.new('sha1')`. Context clues such as variable names containing `password`, `passwd`, or `pwd`, or the hashed value being stored or compared, increase confidence that this is password hashing.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
  (#eq? @module "hashlib")
  (#match? @method "^(md5|sha1)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_password_hash`
- **Severity**: error
- **Confidence**: high
