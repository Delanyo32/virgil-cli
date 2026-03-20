# Type Confusion -- PHP

## Overview
PHP's loose type system performs implicit type juggling in comparisons and function arguments, leading to some of the most exploited type confusion vulnerabilities in web applications. The loose comparison operator `==` follows complex coercion rules that can bypass authentication checks, and functions like `strcmp()` return unexpected values when given non-string inputs. These behaviors are well-documented exploitation techniques in PHP security.

## Why It's a Security Concern
PHP type juggling is a proven attack vector. The comparison `"0e12345" == "0e67890"` evaluates to `true` because both strings are interpreted as zero in scientific notation -- this has been used to bypass password hash comparisons. The comparison `0 == "any_string"` is `true` in PHP < 8.0, enabling authentication bypasses. `strcmp()` returns `0` (meaning "equal") when passed an array instead of a string, which attackers exploit by sending `password[]=` in POST data to bypass `strcmp()`-based password checks.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: WordPress, Laravel, Symfony, any PHP application using `==` for security checks or `strcmp()` for password validation

---

## Pattern 1: Type Juggling via Loose Comparison

### Description
Using the loose comparison operator `==` to compare passwords, hashes, tokens, or other security-critical values. PHP's `==` coerces operands to a common type before comparing. Strings that look like numbers in scientific notation (e.g., `"0e462097431906509019562988736854"`) are coerced to float `0`. Two different MD5 hashes that both start with `0e` followed by digits will compare as equal. Integer `0` equals any non-numeric string in PHP < 8.0.

### Bad Code (Anti-pattern)
```php
function verifyPassword($input, $storedHash) {
    $inputHash = md5($input);
    // Type juggling: "0e12345..." == "0e67890..." is true (both are float 0)
    if ($inputHash == $storedHash) {
        return true; // authentication bypass
    }
    return false;
}

function checkApiKey($request) {
    $provided = $request->header('X-API-Key');
    $expected = config('app.api_key');
    // If $provided is integer 0 (from type juggling), 0 == "secret_key" is true (PHP < 8.0)
    if ($provided == $expected) {
        return true;
    }
    return false;
}

function verifyToken($userToken, $validToken) {
    // Loose comparison: null == false == 0 == "" == "0" are all true
    return ($userToken == $validToken);
}
```

### Good Code (Fix)
```php
function verifyPassword($input, $storedHash) {
    $inputHash = md5($input);
    // Strict comparison -- no type coercion
    if (hash_equals($storedHash, $inputHash)) {
        return true;
    }
    return false;
}

function checkApiKey($request) {
    $provided = $request->header('X-API-Key');
    $expected = config('app.api_key');
    // Strict comparison with type checking
    if (is_string($provided) && hash_equals($expected, $provided)) {
        return true;
    }
    return false;
}

function verifyToken($userToken, $validToken) {
    // Use hash_equals for timing-safe, type-strict comparison
    if (!is_string($userToken) || !is_string($validToken)) {
        return false;
    }
    return hash_equals($validToken, $userToken);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `binary_expression`, `if_statement`, `return_statement`, `variable_name`
- **Detection approach**: Find `binary_expression` nodes with operator `==` or `!=` (not `===` or `!==`). Flag when the comparison involves variables with security-related names (`password`, `hash`, `token`, `key`, `secret`, `auth`, `session`). Also flag `==` comparisons inside `if` conditions that guard `return true` or access-granting logic. Higher confidence when the compared values are results of `md5()`, `sha1()`, or `hash()` calls.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (_) @lhs
  operator: "=="
  right: (_) @rhs)
```

### Pipeline Mapping
- **Pipeline name**: `type_juggling`
- **Pattern name**: `loose_comparison_auth`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: strcmp() Bypass with Non-String Input

### Description
Using `strcmp()` to compare passwords or tokens without validating that both arguments are strings. When `strcmp()` receives an array instead of a string, it returns `NULL` (with a warning in PHP < 8.0, or a `TypeError` in PHP 8.0+). In PHP < 8.0, `NULL == 0` is `true`, so `strcmp($input, $password) == 0` passes when `$input` is an array. Attackers exploit this by sending `password[]=anything` in POST data.

### Bad Code (Anti-pattern)
```php
function login($username, $password) {
    $storedPassword = getPasswordFromDb($username);

    // Attacker sends POST: password[]=x
    // strcmp(array, string) returns NULL in PHP < 8.0
    // NULL == 0 is true -- authentication bypass
    if (strcmp($password, $storedPassword) == 0) {
        createSession($username);
        return true;
    }
    return false;
}

function verifyAnswer($userAnswer, $correctAnswer) {
    // Same vulnerability: strcmp with array input
    if (strcmp($userAnswer, $correctAnswer) == 0) {
        return true;
    }
    return false;
}
```

### Good Code (Fix)
```php
function login($username, $password) {
    // Validate input types first
    if (!is_string($password) || !is_string($username)) {
        return false;
    }

    $storedPassword = getPasswordFromDb($username);
    if ($storedPassword === false) {
        return false;
    }

    // Use strict comparison with hash_equals for timing safety
    if (hash_equals($storedPassword, $password)) {
        createSession($username);
        return true;
    }
    return false;
}

function verifyAnswer($userAnswer, $correctAnswer) {
    if (!is_string($userAnswer)) {
        return false;
    }
    // Use strict comparison operator
    return $userAnswer === $correctAnswer;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `function_call_expression`, `binary_expression`, `name`
- **Detection approach**: Find `function_call_expression` nodes calling `strcmp` (or `strncmp`, `strcasecmp`) where the result is compared using loose equality `== 0`. The pattern `strcmp($a, $b) == 0` is the vulnerable construct; the fix is either `=== 0` (strict) or replacing `strcmp` entirely with `hash_equals` or `===`. Flag with high confidence when the variable names suggest authentication context.
- **S-expression query sketch**:
```scheme
(binary_expression
  left: (function_call_expression
    function: (name) @func_name
    (#eq? @func_name "strcmp"))
  operator: "=="
  right: (integer) @zero)
```

### Pipeline Mapping
- **Pipeline name**: `type_juggling`
- **Pattern name**: `strcmp_bypass`
- **Severity**: error
- **Confidence**: high
