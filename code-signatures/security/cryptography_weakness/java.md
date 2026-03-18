# Cryptography Weakness -- Java

## Overview
Cryptographic weaknesses in Java occur when developers use `java.util.Random` instead of `java.security.SecureRandom` for security-sensitive operations, or rely on weak/deprecated algorithms like MD5, SHA-1, DES, or DESede (Triple DES) for cryptographic purposes. Java's standard library provides both secure and insecure variants side by side, and the insecure variants are often used out of habit or for perceived performance benefits.

## Why It's a Security Concern
`java.util.Random` uses a linear congruential generator with a 48-bit seed -- its entire output sequence can be reconstructed from just two observed values. Tokens, session IDs, and cryptographic keys generated with `Random` are trivially predictable. MD5 and SHA-1 are cryptographically broken (practical collision attacks exist), making them unsuitable for digital signatures, certificate validation, or password storage. DES uses a 56-bit key that can be brute-forced in hours, and DESede (3DES) is deprecated by NIST due to the Sweet32 birthday attack.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.util.Random, java.security.SecureRandom, java.security.MessageDigest, javax.crypto.Cipher, Spring Security, BCrypt, Argon2

---

## Pattern 1: Using java.util.Random Instead of SecureRandom

### Description
Instantiating `java.util.Random` (or its subclass `ThreadLocalRandom` in security contexts) to generate tokens, passwords, session identifiers, encryption keys, nonces, or salts. `Random` uses a linear congruential formula that is entirely predictable given a small sample of outputs. `SecureRandom` draws from the OS entropy pool and is the only appropriate choice for security-sensitive randomness in Java.

### Bad Code (Anti-pattern)
```java
import java.util.Random;

public class TokenGenerator {
    private static final Random random = new Random();

    public static String generateToken(int length) {
        String chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        StringBuilder token = new StringBuilder();
        for (int i = 0; i < length; i++) {
            token.append(chars.charAt(random.nextInt(chars.length())));
        }
        return token.toString();
    }

    public static byte[] generateKey() {
        byte[] key = new byte[16];
        new Random().nextBytes(key);
        return key;
    }
}
```

### Good Code (Fix)
```java
import java.security.SecureRandom;

public class TokenGenerator {
    private static final SecureRandom secureRandom = new SecureRandom();

    public static String generateToken(int length) {
        String chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        StringBuilder token = new StringBuilder();
        for (int i = 0; i < length; i++) {
            token.append(chars.charAt(secureRandom.nextInt(chars.length())));
        }
        return token.toString();
    }

    public static byte[] generateKey() {
        byte[] key = new byte[16];
        secureRandom.nextBytes(key);
        return key;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `type_identifier`, `field_declaration`, `local_variable_declaration`
- **Detection approach**: Find `object_creation_expression` nodes instantiating `Random` (i.e., `new Random()` or `new Random(seed)`). Also detect field or variable declarations with type `Random`. Exclude `SecureRandom` which is a subclass. The presence of `java.util.Random` in imports or as a declared type is the primary signal.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (type_identifier) @type
  (#eq? @type "Random"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `java_util_random`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using MD5/SHA-1/DES/DESede -- Weak Cryptographic Algorithms

### Description
Using `MessageDigest.getInstance("MD5")`, `MessageDigest.getInstance("SHA-1")`, `Cipher.getInstance("DES")`, or `Cipher.getInstance("DESede")` for cryptographic operations. MD5 and SHA-1 have known collision attacks; DES has a 56-bit key vulnerable to brute force; DESede (Triple DES) is deprecated by NIST (2023). These algorithms must be replaced with SHA-256+ for hashing and AES for encryption.

### Bad Code (Anti-pattern)
```java
import java.security.MessageDigest;
import javax.crypto.Cipher;
import javax.crypto.spec.SecretKeySpec;

public class CryptoUtils {
    public static byte[] hashPassword(String password) throws Exception {
        MessageDigest md = MessageDigest.getInstance("MD5");
        return md.digest(password.getBytes());
    }

    public static byte[] encrypt(byte[] data, byte[] key) throws Exception {
        Cipher cipher = Cipher.getInstance("DES/ECB/PKCS5Padding");
        SecretKeySpec keySpec = new SecretKeySpec(key, "DES");
        cipher.init(Cipher.ENCRYPT_MODE, keySpec);
        return cipher.doFinal(data);
    }
}
```

### Good Code (Fix)
```java
import java.security.MessageDigest;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.mindrot.jbcrypt.BCrypt;

public class CryptoUtils {
    public static String hashPassword(String password) {
        return BCrypt.hashpw(password, BCrypt.gensalt(12));
    }

    public static byte[] encrypt(byte[] data, byte[] key, byte[] iv) throws Exception {
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        SecretKeySpec keySpec = new SecretKeySpec(key, "AES");
        GCMParameterSpec gcmSpec = new GCMParameterSpec(128, iv);
        cipher.init(Cipher.ENCRYPT_MODE, keySpec, gcmSpec);
        return cipher.doFinal(data);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `string_literal`, `identifier`
- **Detection approach**: Find `method_invocation` nodes calling `getInstance` on `MessageDigest` or `Cipher` where the first argument is a `string_literal` containing `"MD5"`, `"SHA-1"`, `"SHA1"`, `"DES"`, or `"DESede"`. The string argument directly identifies the weak algorithm. Also detect `SecretKeySpec` constructor calls with `"DES"` as the algorithm parameter.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @class
  name: (identifier) @method
  arguments: (argument_list
    (string_literal) @algo)
  (#match? @class "^(MessageDigest|Cipher)$")
  (#eq? @method "getInstance")
  (#match? @algo "MD5|SHA-1|SHA1|DES|DESede"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_algorithm`
- **Severity**: error
- **Confidence**: high
