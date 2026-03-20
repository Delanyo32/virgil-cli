# Cryptography Weakness -- C#

## Overview
Cryptographic weaknesses in C# occur when developers use `System.Random` instead of `System.Security.Cryptography.RandomNumberGenerator` for security-sensitive operations, or rely on deprecated/weak algorithms like MD5, SHA1, or DES for cryptographic purposes. The .NET framework provides both secure and insecure primitives, and the insecure variants are often chosen for their simpler API or familiarity.

## Why It's a Security Concern
`System.Random` uses a subtraction-based PRNG with a 32-bit seed -- its output is predictable and can be reconstructed from a small number of observations. Tokens, keys, and session IDs generated with `System.Random` are trivially guessable. `MD5CryptoServiceProvider`, `SHA1CryptoServiceProvider`, and `DESCryptoServiceProvider` implement algorithms with known cryptographic breaks: MD5 and SHA1 have practical collision attacks, and DES has a 56-bit key that can be brute-forced in hours. Microsoft has deprecated these in .NET security guidance.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Random, System.Security.Cryptography (RandomNumberGenerator, MD5, SHA1, SHA256, DES, Aes), BCrypt.Net, ASP.NET Identity

---

## Pattern 1: Using System.Random Instead of RandomNumberGenerator

### Description
Instantiating `System.Random` (or `new Random()`) to generate tokens, passwords, encryption keys, nonces, salts, or session identifiers. `System.Random` uses a deterministic algorithm and is not cryptographically secure. `RandomNumberGenerator.GetBytes()` (or `RandomNumberGenerator.GetInt32()` in .NET 6+) provides cryptographically strong random values from the OS entropy pool.

### Bad Code (Anti-pattern)
```csharp
using System;

public class TokenService
{
    private static readonly Random random = new Random();

    public static string GenerateToken(int length)
    {
        const string chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        var token = new char[length];
        for (int i = 0; i < length; i++)
        {
            token[i] = chars[random.Next(chars.Length)];
        }
        return new string(token);
    }

    public static byte[] GenerateKey()
    {
        var key = new byte[32];
        new Random().NextBytes(key);
        return key;
    }
}
```

### Good Code (Fix)
```csharp
using System.Security.Cryptography;

public class TokenService
{
    public static string GenerateToken(int length)
    {
        const string chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        var token = new char[length];
        for (int i = 0; i < length; i++)
        {
            token[i] = chars[RandomNumberGenerator.GetInt32(chars.Length)];
        }
        return new string(token);
    }

    public static byte[] GenerateKey()
    {
        var key = new byte[32];
        RandomNumberGenerator.Fill(key);
        return key;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `identifier`, `variable_declaration`, `field_declaration`
- **Detection approach**: Find `object_creation_expression` nodes instantiating `Random` (i.e., `new Random()` or `new Random(seed)`). Also detect field or variable declarations with type `Random`. Exclude `RandomNumberGenerator` which is the secure alternative. The type name `Random` in a `using System;` context is the primary signal.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (identifier) @type
  (#eq? @type "Random"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `system_random`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Using MD5/SHA1/DES for Cryptographic Purposes

### Description
Using `MD5.Create()`, `SHA1.Create()`, `MD5CryptoServiceProvider`, `SHA1CryptoServiceProvider`, `DESCryptoServiceProvider`, or `TripleDESCryptoServiceProvider` for password hashing, data integrity, or encryption. These algorithms are cryptographically broken or deprecated. Use `SHA256`/`SHA512` for hashing, `Aes` for encryption, and `BCrypt`/`Argon2` or ASP.NET Identity's `PasswordHasher` for password storage.

### Bad Code (Anti-pattern)
```csharp
using System.Security.Cryptography;
using System.Text;

public class CryptoUtils
{
    public static string HashPassword(string password)
    {
        using var md5 = MD5.Create();
        byte[] hash = md5.ComputeHash(Encoding.UTF8.GetBytes(password));
        return BitConverter.ToString(hash).Replace("-", "").ToLower();
    }

    public static byte[] Encrypt(byte[] data, byte[] key)
    {
        using var des = DES.Create();
        des.Key = key;
        using var encryptor = des.CreateEncryptor();
        return encryptor.TransformFinalBlock(data, 0, data.Length);
    }
}
```

### Good Code (Fix)
```csharp
using System.Security.Cryptography;
using System.Text;
using BCrypt.Net;

public class CryptoUtils
{
    public static string HashPassword(string password)
    {
        return BCrypt.Net.BCrypt.HashPassword(password, workFactor: 12);
    }

    public static byte[] Encrypt(byte[] data, byte[] key, byte[] iv)
    {
        using var aes = Aes.Create();
        aes.Key = key;
        aes.IV = iv;
        using var encryptor = aes.CreateEncryptor();
        return encryptor.TransformFinalBlock(data, 0, data.Length);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `identifier`, `object_creation_expression`
- **Detection approach**: Find `invocation_expression` nodes calling `MD5.Create()`, `SHA1.Create()`, or `DES.Create()`. Also find `object_creation_expression` nodes instantiating `MD5CryptoServiceProvider`, `SHA1CryptoServiceProvider`, `DESCryptoServiceProvider`, or `TripleDESCryptoServiceProvider`. The class name directly identifies the weak algorithm.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (member_access_expression
    expression: (identifier) @class
    name: (identifier) @method)
  (#match? @class "^(MD5|SHA1|DES)$")
  (#eq? @method "Create"))
```

### Pipeline Mapping
- **Pipeline name**: `weak_cryptography`
- **Pattern name**: `weak_algorithm`
- **Severity**: error
- **Confidence**: high
