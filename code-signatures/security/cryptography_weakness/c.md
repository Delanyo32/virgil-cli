# Cryptography Weakness -- C

## Overview
Cryptographic weaknesses in C arise from using the standard library's `rand()`/`srand()` functions for security-sensitive operations, or relying on deprecated cryptographic algorithms like DES and MD5 via OpenSSL or similar libraries. C's standard `rand()` is one of the weakest PRNGs in any language's standard library -- typically a linear congruential generator with as few as 32 bits of state -- and its use for tokens, keys, or nonces is a critical vulnerability.

## Why It's a Security Concern
`rand()` seeded with `srand(time(NULL))` produces output with at most 32 bits of entropy, and the seed can be guessed if the approximate time of execution is known. Many implementations use a linear congruential generator whose full state can be recovered from a single output value. DES encryption uses a 56-bit key that can be brute-forced in hours on modern hardware. MD5 has practical collision attacks (since 2004) and pre-image resistance concerns, making it unsuitable for certificates, digital signatures, or password storage.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libc (stdlib.h), OpenSSL (libcrypto), libsodium, /dev/urandom, getrandom(2)

---

## Pattern 1: Using rand()/srand() Instead of /dev/urandom or getrandom()

### Description
Using `rand()`, `srand()`, or `random()`/`srandom()` from `<stdlib.h>` to generate security-sensitive values such as tokens, session identifiers, encryption keys, nonces, or salts. These functions use a deterministic PRNG with limited state and are trivially predictable. The correct alternatives are `getrandom()` (Linux 3.17+), reading from `/dev/urandom`, or using a library like libsodium's `randombytes_buf()`.

### Bad Code (Anti-pattern)
```c
#include <stdlib.h>
#include <time.h>
#include <string.h>

void generate_session_token(char *token, size_t len) {
    srand(time(NULL));
    const char charset[] = "abcdefghijklmnopqrstuvwxyz0123456789";
    for (size_t i = 0; i < len - 1; i++) {
        token[i] = charset[rand() % (sizeof(charset) - 1)];
    }
    token[len - 1] = '\0';
}

void generate_key(unsigned char *key, size_t len) {
    srand(time(NULL) ^ getpid());
    for (size_t i = 0; i < len; i++) {
        key[i] = rand() & 0xFF;
    }
}
```

### Good Code (Fix)
```c
#include <sys/random.h>
#include <string.h>

void generate_session_token(char *token, size_t len) {
    unsigned char buf[256];
    getrandom(buf, len - 1, 0);
    const char charset[] = "abcdefghijklmnopqrstuvwxyz0123456789";
    for (size_t i = 0; i < len - 1; i++) {
        token[i] = charset[buf[i] % (sizeof(charset) - 1)];
    }
    token[len - 1] = '\0';
}

void generate_key(unsigned char *key, size_t len) {
    getrandom(key, len, 0);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes calling `rand`, `srand`, `random`, or `srandom`. These functions are inherently non-cryptographic and should be flagged in any codebase that performs security-sensitive operations. The presence of `srand(time(NULL))` is a particularly strong signal of misuse for security purposes.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  (#match? @func "^(rand|srand|random|srandom)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `rand_srand_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using DES/MD5 via OpenSSL

### Description
Using `DES_ecb_encrypt()`, `DES_cbc_encrypt()`, `MD5()`, `MD5_Init()`/`MD5_Update()`/`MD5_Final()`, or `EVP_md5()`/`EVP_des_cbc()` from OpenSSL for cryptographic operations. DES has a 56-bit key that is trivially brute-forced, and MD5 has practical collision attacks. These have been deprecated in OpenSSL 3.0's default provider. Use AES-256-GCM for encryption and SHA-256+ for hashing.

### Bad Code (Anti-pattern)
```c
#include <openssl/des.h>
#include <openssl/md5.h>

void hash_password(const char *password, unsigned char *digest) {
    MD5((const unsigned char *)password, strlen(password), digest);
}

void encrypt_data(const unsigned char *input, unsigned char *output,
                  DES_key_schedule *ks) {
    DES_ecb_encrypt((const_DES_cblock *)input, (DES_cblock *)output,
                    ks, DES_ENCRYPT);
}
```

### Good Code (Fix)
```c
#include <openssl/evp.h>
#include <openssl/sha.h>

void hash_password(const char *password, unsigned char *digest) {
    EVP_MD_CTX *ctx = EVP_MD_CTX_new();
    EVP_DigestInit_ex(ctx, EVP_sha256(), NULL);
    EVP_DigestUpdate(ctx, password, strlen(password));
    EVP_DigestFinal_ex(ctx, digest, NULL);
    EVP_MD_CTX_free(ctx);
}

int encrypt_data(const unsigned char *input, int input_len,
                 const unsigned char *key, const unsigned char *iv,
                 unsigned char *output) {
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    int len, ciphertext_len;
    EVP_EncryptInit_ex(ctx, EVP_aes_256_gcm(), NULL, key, iv);
    EVP_EncryptUpdate(ctx, output, &len, input, input_len);
    ciphertext_len = len;
    EVP_EncryptFinal_ex(ctx, output + len, &len);
    ciphertext_len += len;
    EVP_CIPHER_CTX_free(ctx);
    return ciphertext_len;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes calling `MD5`, `MD5_Init`, `MD5_Update`, `MD5_Final`, `DES_ecb_encrypt`, `DES_cbc_encrypt`, `DES_ncbc_encrypt`, `DES_set_key`, or EVP functions with weak algorithm parameters like `EVP_md5()` or `EVP_des_cbc()`. Also detect `#include <openssl/des.h>` or `#include <openssl/md5.h>` as indicators.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  (#match? @func "^(MD5|MD5_Init|MD5_Update|MD5_Final|DES_ecb_encrypt|DES_cbc_encrypt|DES_ncbc_encrypt|DES_set_key)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `des_md5_openssl`
- **Severity**: error
- **Confidence**: high
