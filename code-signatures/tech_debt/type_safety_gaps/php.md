# Type Safety Gaps -- PHP

## Overview
PHP has supported type declarations since PHP 7.0 (scalar types) and PHP 7.1 (nullable, void), with union types added in 8.0. However, many codebases still contain functions without type declarations and functions that return mixed types without declaring union types, undermining static analysis tools like PHPStan and Psalm.

## Why It's a Tech Debt Concern
Functions without type declarations on parameters and return values rely entirely on runtime behavior and documentation for correctness, preventing static analysis tools from catching type mismatches before deployment. Functions that return different types depending on conditions (e.g., `string` or `false`, `array` or `null`) without declaring a union return type create invisible API contracts that callers must discover through trial and error or source code inspection.

## Applicability
- **Relevance**: high (PHP's gradual typing adoption means many codebases have partial coverage)
- **Languages covered**: `.php`
- **Frameworks/libraries**: Laravel, Symfony, WordPress, legacy PHP applications

---

## Pattern 1: Missing Type Declarations on Function Parameters/Returns

### Description
Functions and methods that lack type declarations on one or more parameters or on the return type. Without these declarations, PHP performs no type checking and static analysis tools cannot verify call-site correctness. This is especially problematic for public API methods.

### Bad Code (Anti-pattern)
```php
class UserRepository
{
    public function findById($id)
    {
        $stmt = $this->db->prepare('SELECT * FROM users WHERE id = ?');
        $stmt->execute([$id]);
        return $stmt->fetch();
    }

    public function save($user)
    {
        if ($user->id) {
            return $this->update($user);
        }
        return $this->insert($user);
    }

    public function search($query, $limit, $offset)
    {
        $stmt = $this->db->prepare('SELECT * FROM users WHERE name LIKE ? LIMIT ? OFFSET ?');
        $stmt->execute(["%$query%", $limit, $offset]);
        return $stmt->fetchAll();
    }
}

function formatPrice($amount, $currency, $locale)
{
    $formatter = new NumberFormatter($locale, NumberFormatter::CURRENCY);
    return $formatter->formatCurrency($amount, $currency);
}
```

### Good Code (Fix)
```php
class UserRepository
{
    public function findById(int $id): ?array
    {
        $stmt = $this->db->prepare('SELECT * FROM users WHERE id = ?');
        $stmt->execute([$id]);
        $result = $stmt->fetch();
        return $result !== false ? $result : null;
    }

    public function save(User $user): int
    {
        if ($user->id) {
            return $this->update($user);
        }
        return $this->insert($user);
    }

    public function search(string $query, int $limit = 20, int $offset = 0): array
    {
        $stmt = $this->db->prepare('SELECT * FROM users WHERE name LIKE ? LIMIT ? OFFSET ?');
        $stmt->execute(["%$query%", $limit, $offset]);
        return $stmt->fetchAll();
    }
}

function formatPrice(float $amount, string $currency, string $locale): string
{
    $formatter = new NumberFormatter($locale, NumberFormatter::CURRENCY);
    return $formatter->formatCurrency($amount, $currency);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `method_declaration`, `simple_parameter`, `type`, `union_type`
- **Detection approach**: Find `function_definition` and `method_declaration` nodes. For each `simple_parameter` child, check whether it has a `type` child node. Also check whether the function/method has a return `type` annotation (`:` followed by a type before the body). Flag functions where any parameter lacks a type declaration or where the return type is absent.
- **S-expression query sketch**:
```scheme
(function_definition
  name: (name) @func_name
  parameters: (formal_parameters
    (simple_parameter
      name: (variable_name) @param_name)))

(method_declaration
  name: (name) @method_name
  parameters: (formal_parameters
    (simple_parameter
      name: (variable_name) @param_name)))
```

### Pipeline Mapping
- **Pipeline name**: `missing_type_declarations`
- **Pattern name**: `untyped_parameter_or_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Return Types Without Union Type Declaration

### Description
Functions that return different types depending on code paths (e.g., returning a `string` on success and `false` on failure, or `array` on success and `null` on error) without declaring a union return type. Callers cannot know from the signature alone what types to expect, leading to unchecked return values and type errors.

### Bad Code (Anti-pattern)
```php
class FileProcessor
{
    public function readConfig(string $path)
    {
        if (!file_exists($path)) {
            return false;  // Returns bool
        }
        $content = file_get_contents($path);
        if ($content === false) {
            return null;   // Returns null
        }
        return json_decode($content, true);  // Returns array
    }

    public function parseValue(string $input)
    {
        if (is_numeric($input)) {
            return (int) $input;     // Returns int
        }
        if ($input === 'true' || $input === 'false') {
            return $input === 'true'; // Returns bool
        }
        return $input;               // Returns string
    }

    public function findRecord(string $table, int $id)
    {
        $result = $this->db->fetch($table, $id);
        if (!$result) {
            return -1;  // Returns int as error code
        }
        return $result;  // Returns array
    }
}
```

### Good Code (Fix)
```php
class FileProcessor
{
    public function readConfig(string $path): array|null
    {
        if (!file_exists($path)) {
            return null;
        }
        $content = file_get_contents($path);
        if ($content === false) {
            return null;
        }
        $decoded = json_decode($content, true);
        return is_array($decoded) ? $decoded : null;
    }

    public function parseValue(string $input): int|bool|string
    {
        if (is_numeric($input)) {
            return (int) $input;
        }
        if ($input === 'true' || $input === 'false') {
            return $input === 'true';
        }
        return $input;
    }

    public function findRecord(string $table, int $id): ?array
    {
        $result = $this->db->fetch($table, $id);
        if (!$result) {
            return null;  // Consistent null instead of magic -1
        }
        return $result;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `method_declaration`, `return_statement`, `return_type`
- **Detection approach**: Find `function_definition` and `method_declaration` nodes. Collect all `return_statement` nodes within the body. Analyze the return expressions to determine if different return statements yield different types (e.g., one returns a `string` literal, another returns `false`, another returns `null`). Flag functions with multiple distinct return types that lack a `union_type` or `nullable_type` return type declaration.
- **S-expression query sketch**:
```scheme
(function_definition
  name: (name) @func_name
  return_type: (_)? @ret_type
  body: (compound_statement
    (return_statement
      (_) @return_expr)))

(method_declaration
  name: (name) @method_name
  return_type: (_)? @ret_type
  body: (compound_statement
    (return_statement
      (_) @return_expr)))
```

### Pipeline Mapping
- **Pipeline name**: `missing_type_declarations`
- **Pattern name**: `mixed_return_types`
- **Severity**: warning
- **Confidence**: medium
