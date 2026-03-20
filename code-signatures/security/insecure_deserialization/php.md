# Insecure Deserialization -- PHP

## Overview
Insecure deserialization in PHP centers on the `unserialize()` function, which can instantiate arbitrary objects and trigger magic methods (`__wakeup`, `__destruct`, `__toString`) during deserialization. This is a well-known PHP object injection vector. Additionally, `json_decode()` without validation can lead to type confusion and logic bypass.

## Why It's a Security Concern
PHP's `unserialize()` reconstructs objects from serialized strings, invoking magic methods that may perform dangerous operations (file deletion, database queries, command execution). Attackers craft serialized payloads using classes available in the application or its dependencies (POP chains) to achieve remote code execution. Even `json_decode()` without validation can introduce unexpected types and values.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: PHP core, Laravel, Symfony, WordPress, Magento, Doctrine

---

## Pattern 1: unserialize() on User Input

### Description
Calling `unserialize()` on data that originates from user input (cookies, POST data, URL parameters, database fields populated by users). PHP object injection allows attackers to instantiate any autoloaded class and trigger its magic methods, potentially leading to RCE.

### Bad Code (Anti-pattern)
```php
function loadPreferences($request) {
    $prefs = unserialize($request->cookie('user_prefs')); // RCE via object injection
    return $prefs;
}
```

### Good Code (Fix)
```php
function loadPreferences($request) {
    $prefs = json_decode($request->cookie('user_prefs'), true);
    // Validate the decoded array
    if (!is_array($prefs) || !isset($prefs['theme'], $prefs['lang'])) {
        return getDefaultPreferences();
    }
    return array_intersect_key($prefs, array_flip(['theme', 'lang', 'timezone']));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `arguments`
- **Detection approach**: Find `function_call_expression` nodes where the function name is `unserialize`. Flag all usages since `unserialize()` on any external data is dangerous. Check if the `allowed_classes` option (second parameter as array with `'allowed_classes' => false`) is provided — its absence increases severity.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `unserialize_user_input`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: json_decode() Without Validation

### Description
Calling `json_decode()` on untrusted input and using the result without validating the structure, types, or values. While `json_decode()` cannot execute code, the lack of validation can lead to type confusion (object vs. array), null reference errors, or logic bypass when attackers supply unexpected JSON shapes.

### Bad Code (Anti-pattern)
```php
function processWebhook($rawBody) {
    $payload = json_decode($rawBody);
    $amount = $payload->amount;  // No type check — could be string, null, or object
    $this->chargeAccount($payload->account_id, $amount);
}
```

### Good Code (Fix)
```php
function processWebhook($rawBody) {
    $payload = json_decode($rawBody, true);
    if (!is_array($payload)) {
        throw new \InvalidArgumentException('Invalid payload');
    }
    $amount = filter_var($payload['amount'] ?? null, FILTER_VALIDATE_FLOAT);
    $accountId = filter_var($payload['account_id'] ?? null, FILTER_VALIDATE_INT);
    if ($amount === false || $accountId === false || $amount <= 0) {
        throw new \InvalidArgumentException('Invalid payload values');
    }
    $this->chargeAccount($accountId, $amount);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `member_access_expression`
- **Detection approach**: Find `function_call_expression` nodes calling `json_decode`. Flag when the return value is used directly in member access, array access, or passed to functions without an intervening validation check (e.g., `is_array()`, `isset()`, `filter_var()`).
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments (_) @input))
```

### Pipeline Mapping
- **Pipeline name**: `insecure_deserialization`
- **Pattern name**: `json_decode_no_validation`
- **Severity**: warning
- **Confidence**: medium
