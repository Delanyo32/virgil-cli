# Cryptography Weakness -- C++

## Overview
Cryptographic weaknesses in C++ arise from using `std::rand()`/`std::srand()` or the C `rand()` function for security-sensitive operations, or relying on deprecated cryptographic functions inherited from C libraries. C++'s `<random>` header provides `std::random_device` for non-deterministic random number generation, but many developers default to the simpler (and insecure) `rand()`/`srand()` pair, especially in legacy codebases.

## Why It's a Security Concern
`std::rand()` and `std::srand()` inherit the same weaknesses as C's `rand()` -- a linear congruential generator with limited state, trivially predictable output, and `srand(time(nullptr))` providing at most 32 bits of entropy. Even C++'s `std::mt19937` (Mersenne Twister) is not cryptographically secure -- its full state can be recovered from 624 consecutive 32-bit outputs. Using deprecated crypto functions (MD5, DES via OpenSSL or Crypto++) in C++ applications exposes the same vulnerabilities as in C: practical collision attacks on MD5, brute-forceable DES keys, and pre-image weaknesses.

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: C++ Standard Library (<random>, <cstdlib>), OpenSSL (libcrypto), Crypto++, Botan, libsodium

---

## Pattern 1: Using std::rand()/std::srand() Instead of std::random_device for Security

### Description
Using `std::rand()`, `std::srand()`, `rand()`, or `srand()` from `<cstdlib>` to generate security-sensitive values such as tokens, encryption keys, nonces, or salts. Also includes using `std::mt19937` seeded with `std::random_device` for non-cryptographic engines in security contexts. The correct approach for cryptographic randomness is to use `std::random_device` directly (which wraps the OS CSPRNG on all major platforms), or a library like libsodium's `randombytes_buf()`.

### Bad Code (Anti-pattern)
```cpp
#include <cstdlib>
#include <ctime>
#include <string>

std::string generate_token(size_t length) {
    std::srand(std::time(nullptr));
    const std::string charset = "abcdefghijklmnopqrstuvwxyz0123456789";
    std::string token;
    for (size_t i = 0; i < length; i++) {
        token += charset[std::rand() % charset.size()];
    }
    return token;
}

void generate_key(unsigned char* key, size_t len) {
    srand(time(nullptr) ^ getpid());
    for (size_t i = 0; i < len; i++) {
        key[i] = rand() & 0xFF;
    }
}
```

### Good Code (Fix)
```cpp
#include <random>
#include <string>
#include <algorithm>

std::string generate_token(size_t length) {
    std::random_device rd;
    const std::string charset = "abcdefghijklmnopqrstuvwxyz0123456789";
    std::string token(length, '\0');
    std::uniform_int_distribution<size_t> dist(0, charset.size() - 1);
    for (size_t i = 0; i < length; i++) {
        token[i] = charset[dist(rd)];
    }
    return token;
}

void generate_key(unsigned char* key, size_t len) {
    std::random_device rd;
    for (size_t i = 0; i < len; i += sizeof(unsigned int)) {
        unsigned int val = rd();
        size_t bytes = std::min(sizeof(unsigned int), len - i);
        std::memcpy(key + i, &val, bytes);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `qualified_identifier`, `identifier`, `scope_resolution`
- **Detection approach**: Find `call_expression` nodes calling `std::rand`, `std::srand`, `rand`, or `srand`. Also detect `#include <cstdlib>` combined with calls to `rand()`/`srand()`. The `std::` prefix or bare function call from C's `<stdlib.h>` both indicate the insecure variant. Context of token/key generation in surrounding code increases confidence.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (qualified_identifier
    scope: (namespace_identifier) @ns
    name: (identifier) @func)
  (#eq? @ns "std")
  (#match? @func "^(rand|srand)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `std_rand_security`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using Deprecated Cryptographic Functions

### Description
Using deprecated OpenSSL functions such as `MD5()`, `MD5_Init()`/`MD5_Update()`/`MD5_Final()`, `DES_ecb_encrypt()`, `SHA1()`, or their Crypto++ equivalents for security purposes. Also includes using `EVP_md5()`, `EVP_des_cbc()`, or `EVP_sha1()` as algorithm parameters. These algorithms have known cryptographic weaknesses and are deprecated in OpenSSL 3.0's default provider. Modern C++ code should use AES-256-GCM for encryption and SHA-256/SHA-512 for hashing.

### Bad Code (Anti-pattern)
```cpp
#include <openssl/md5.h>
#include <openssl/des.h>
#include <string>
#include <cstring>

std::string hash_password(const std::string& password) {
    unsigned char digest[MD5_DIGEST_LENGTH];
    MD5(reinterpret_cast<const unsigned char*>(password.c_str()),
        password.size(), digest);
    char hex[MD5_DIGEST_LENGTH * 2 + 1];
    for (int i = 0; i < MD5_DIGEST_LENGTH; i++) {
        sprintf(hex + i * 2, "%02x", digest[i]);
    }
    return std::string(hex);
}

void encrypt_block(const unsigned char* input, unsigned char* output,
                   DES_key_schedule* ks) {
    DES_ecb_encrypt(reinterpret_cast<const_DES_cblock*>(input),
                    reinterpret_cast<DES_cblock*>(output),
                    ks, DES_ENCRYPT);
}
```

### Good Code (Fix)
```cpp
#include <openssl/evp.h>
#include <string>
#include <vector>
#include <memory>

std::string hash_password(const std::string& password) {
    // Use argon2 or bcrypt for actual password hashing
    // SHA-256 shown here for general hashing only
    unsigned char digest[SHA256_DIGEST_LENGTH];
    auto ctx = std::unique_ptr<EVP_MD_CTX, decltype(&EVP_MD_CTX_free)>(
        EVP_MD_CTX_new(), EVP_MD_CTX_free);
    EVP_DigestInit_ex(ctx.get(), EVP_sha256(), nullptr);
    EVP_DigestUpdate(ctx.get(), password.c_str(), password.size());
    EVP_DigestFinal_ex(ctx.get(), digest, nullptr);
    char hex[SHA256_DIGEST_LENGTH * 2 + 1];
    for (int i = 0; i < SHA256_DIGEST_LENGTH; i++) {
        sprintf(hex + i * 2, "%02x", digest[i]);
    }
    return std::string(hex);
}

std::vector<unsigned char> encrypt_data(const unsigned char* input, int input_len,
                                         const unsigned char* key, const unsigned char* iv) {
    auto ctx = std::unique_ptr<EVP_CIPHER_CTX, decltype(&EVP_CIPHER_CTX_free)>(
        EVP_CIPHER_CTX_new(), EVP_CIPHER_CTX_free);
    int len, ciphertext_len;
    std::vector<unsigned char> output(input_len + EVP_MAX_BLOCK_LENGTH);
    EVP_EncryptInit_ex(ctx.get(), EVP_aes_256_gcm(), nullptr, key, iv);
    EVP_EncryptUpdate(ctx.get(), output.data(), &len, input, input_len);
    ciphertext_len = len;
    EVP_EncryptFinal_ex(ctx.get(), output.data() + len, &len);
    ciphertext_len += len;
    output.resize(ciphertext_len);
    return output;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `qualified_identifier`, `identifier`, `preproc_include`
- **Detection approach**: Find `call_expression` nodes calling `MD5`, `MD5_Init`, `MD5_Update`, `MD5_Final`, `SHA1`, `SHA1_Init`, `DES_ecb_encrypt`, `DES_cbc_encrypt`, `DES_set_key`, or EVP functions with deprecated algorithm parameters. Also detect `#include <openssl/md5.h>`, `#include <openssl/des.h>`, or `#include <openssl/sha.h>` (when SHA-1 specific usage follows).
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func
  (#match? @func "^(MD5|MD5_Init|MD5_Update|MD5_Final|SHA1|SHA1_Init|DES_ecb_encrypt|DES_cbc_encrypt|DES_set_key)$"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_randomness`
- **Pattern name**: `deprecated_crypto_functions`
- **Severity**: error
- **Confidence**: high
