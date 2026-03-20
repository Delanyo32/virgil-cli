# Dead Code -- PHP

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. PHP's dynamic nature and lack of a compilation step make dead code especially hard to detect — unused functions are only caught at runtime or by static analysis tools.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Function/Method

### Description
A function or method defined but never called from anywhere in the codebase.

### Bad Code (Anti-pattern)
```php
class ImageProcessor
{
    private function resizeWithGd($image, $width, $height)
    {
        $resized = imagecreatetruecolor($width, $height);
        imagecopyresampled($resized, $image, 0, 0, 0, 0, $width, $height,
            imagesx($image), imagesy($image));
        return $resized;
    }

    public function resize($path, $width, $height)
    {
        $manager = new ImageManager(new Driver());
        return $manager->read($path)->scale($width, $height);
    }
}
```

### Good Code (Fix)
```php
class ImageProcessor
{
    public function resize($path, $width, $height)
    {
        $manager = new ImageManager(new Driver());
        return $manager->read($path)->scale($width, $height);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `method_declaration`
- **Detection approach**: Collect all function/method definitions and their names. Cross-reference with all `function_call_expression` and `member_call_expression` nodes across the project. Functions with zero references are candidates. Exclude magic methods (`__construct`, `__destruct`, `__get`, `__set`, `__call`, `__toString`, etc.), public methods in classes that implement interfaces, methods referenced in route definitions or service containers, and functions used as callbacks via string references (`'functionName'` or `[$obj, 'method']`).
- **S-expression query sketch**:
  ```scheme
  (function_definition name: (name) @fn_name)
  (method_declaration name: (name) @method_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Throw/Exit

### Description
Code statements that appear after an unconditional return, throw, exit, die, or continue — they can never execute.

### Bad Code (Anti-pattern)
```php
function loadConfiguration(string $path): array
{
    if (!file_exists($path)) {
        throw new RuntimeException("Config file not found: $path");
        error_log("Missing config: $path"); // unreachable
        return []; // unreachable
    }

    $data = json_decode(file_get_contents($path), true);
    if (json_last_error() !== JSON_ERROR_NONE) {
        die('Invalid JSON in config file');
        return []; // unreachable
    }

    return $data;
}
```

### Good Code (Fix)
```php
function loadConfiguration(string $path): array
{
    if (!file_exists($path)) {
        throw new RuntimeException("Config file not found: $path");
    }

    $data = json_decode(file_get_contents($path), true);
    if (json_last_error() !== JSON_ERROR_NONE) {
        die('Invalid JSON in config file');
    }

    return $data;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `throw_expression`, `break_statement`, `continue_statement`, `expression_statement` (for `exit()`, `die()`)
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `compound_statement`. In PHP, `exit()`, `die()`, `throw`, and `return` are diverging. Also check for statements after unconditional `break`/`continue` in loops and switch cases.
- **S-expression query sketch**:
  ```scheme
  (compound_statement
    (return_statement) @exit
    .
    (_) @unreachable)
  (compound_statement
    (throw_expression) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```php
class PaymentGateway
{
    public function charge(Order $order): PaymentResult
    {
        // function validateCardNumber(string $number): bool
        // {
        //     $number = preg_replace('/\D/', '', $number);
        //     $sum = 0;
        //     $alt = false;
        //     for ($i = strlen($number) - 1; $i >= 0; $i--) {
        //         $n = intval($number[$i]);
        //         if ($alt) {
        //             $n *= 2;
        //             if ($n > 9) $n -= 9;
        //         }
        //         $sum += $n;
        //         $alt = !$alt;
        //     }
        //     return $sum % 10 === 0;
        // }

        return $this->gateway->purchase($order->total, $order->paymentMethod);
    }
}
```

### Good Code (Fix)
```php
class PaymentGateway
{
    public function charge(Order $order): PaymentResult
    {
        return $this->gateway->purchase($order->total, $order->paymentMethod);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment`
- **Detection approach**: Find comment nodes whose content matches PHP code patterns (contains `function `, `$`, `->`, `::`, `return `, `if (`, `foreach (`, `new `, semicolons at end of lines). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from PHPDoc blocks (`/** */`), license headers, and annotation comments.
- **S-expression query sketch**:
  ```scheme
  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
