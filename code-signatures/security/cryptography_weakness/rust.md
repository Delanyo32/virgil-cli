# Cryptography Weakness -- Rust

## Overview
Cryptographic weaknesses in Rust occur when developers use general-purpose random number generators like `rand::thread_rng()` for cryptographic operations instead of `OsRng`, or rely on deprecated/weak hash algorithms for security-sensitive purposes. While Rust's type system and memory safety eliminate many vulnerability classes, it cannot prevent the misuse of cryptographic primitives -- choosing the wrong RNG or hash algorithm is a logic error that the compiler cannot catch.

## Why It's a Security Concern
`rand::thread_rng()` uses a fast, high-quality PRNG (ChaCha) that is reseeded from the OS, but it is not designed for all cryptographic contexts -- `OsRng` provides direct access to the operating system's CSPRNG and is the recommended choice for key generation and other security-critical randomness. Using weak or deprecated hash algorithms (MD5, SHA-1) for integrity verification, message authentication, or password storage exposes applications to collision attacks and brute-force cracking.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: rand, rand_core, getrandom, ring, sha2, md-5, sha1, argon2, bcrypt, rust-crypto

---

## Pattern 1: Using rand::thread_rng() Where OsRng Is Needed for Cryptography

### Description
Using `rand::thread_rng()` or `rand::random()` to generate cryptographic keys, nonces, initialization vectors, or other values where the randomness source must be the operating system's CSPRNG. While `thread_rng()` is periodically reseeded from the OS, `OsRng` provides a direct, auditable path to `/dev/urandom` or the platform equivalent, which is the expected primitive in cryptographic protocols and security audits.

### Bad Code (Anti-pattern)
```rust
use rand::Rng;

fn generate_encryption_key() -> [u8; 32] {
    let mut rng = rand::thread_rng();
    let mut key = [0u8; 32];
    rng.fill(&mut key);
    key
}

fn generate_nonce() -> u64 {
    rand::random::<u64>()
}
```

### Good Code (Fix)
```rust
use rand::rngs::OsRng;
use rand::RngCore;

fn generate_encryption_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

fn generate_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    nonce
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `identifier`, `field_expression`
- **Detection approach**: Find `call_expression` nodes where the function is `rand::thread_rng` or `rand::random` (as a `scoped_identifier`). Also detect `use rand::thread_rng` imports followed by bare `thread_rng()` calls. Context clues such as the result being used to fill byte arrays named `key`, `nonce`, `iv`, `secret`, or `salt` increase confidence.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @crate
    name: (identifier) @func)
  (#eq? @crate "rand")
  (#match? @func "^(thread_rng|random)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `thread_rng_crypto`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Using Deprecated or Weak Hash Algorithms

### Description
Using MD5 (`md5` crate or `Md5` from `md-5`) or SHA-1 (`sha1` crate or `Sha1` from `sha1`) for security purposes such as password hashing, message authentication, or digital signatures. MD5 has been cryptographically broken since 2004 (practical collision attacks), and SHA-1 since 2017 (SHAttered attack). These algorithms must not be used where collision resistance or pre-image resistance is a security requirement.

### Bad Code (Anti-pattern)
```rust
use md5;
use sha1::{Sha1, Digest};

fn hash_password(password: &str) -> String {
    let digest = md5::compute(password.as_bytes());
    format!("{:x}", digest)
}

fn verify_integrity(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}
```

### Good Code (Fix)
```rust
use argon2::{self, Config};
use sha2::{Sha256, Digest};

fn hash_password(password: &str) -> String {
    let salt = generate_salt();
    let config = Config::default();
    argon2::hash_encoded(password.as_bytes(), &salt, &config).unwrap()
}

fn verify_integrity(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `use_declaration`, `identifier`
- **Detection approach**: Find `use_declaration` nodes importing from `md5`, `md-5`, or `sha1` crates. Also find `call_expression` nodes calling `md5::compute`, `Md5::new`, or `Sha1::new`. Flag these as weak hash usage. Context of password hashing (variable names, function names containing `password`, `auth`, `credential`) elevates severity.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @crate
    name: (identifier) @func)
  (#match? @crate "^(md5|sha1)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_hash_algorithm`
- **Severity**: error
- **Confidence**: high
