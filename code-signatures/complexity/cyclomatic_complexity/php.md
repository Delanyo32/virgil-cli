# Cyclomatic Complexity -- PHP

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `elseif`, `switch` cases, loops (`for`, `foreach`, `while`, `do-while`), logical operators (`&&`, `||`), ternary expressions (`?:`), and `catch` clauses. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Every decision point adds a branch that demands its own test case, so high-CC functions require disproportionate testing effort. PHP's dynamic typing and loose comparisons can compound the problem by hiding implicit type coercion branches that developers may not anticipate. Elevated cyclomatic complexity correlates with higher defect rates and increases the cost of maintenance and refactoring.

## Applicability
- **Relevance**: high
- **Languages covered**: `.php`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/elseif branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```php
function processPayment(array $payment, array $config): string
{
    if (empty($payment)) {
        return 'invalid';
    }

    if ($payment['type'] === 'subscription') {
        if ($payment['amount'] > 10000 || $config['force_review']) {
            if ($payment['currency'] !== 'USD' && $payment['currency'] !== 'EUR') {
                return 'fx_review';
            } elseif ($payment['customer']['is_vip']) {
                return 'vip_review';
            } else {
                return 'standard_review';
            }
        } elseif ($payment['amount'] > 5000 && $payment['recurring']) {
            return 'recurring_review';
        } else {
            return 'auto_approve';
        }
    } elseif ($payment['type'] === 'one_time') {
        switch ($payment['method']) {
            case 'credit_card':
                if ($payment['amount'] > 1000 && !$payment['verified']) {
                    return 'fraud_check';
                }
                return 'process_card';
            case 'bank_transfer':
                return $payment['domestic'] ? 'process_domestic' : 'process_international';
            case 'paypal':
                return 'process_paypal';
            default:
                return 'unsupported_method';
        }
    } elseif ($payment['type'] === 'refund') {
        if ($payment['original_settled'] && $payment['amount'] <= $payment['original_amount']) {
            return 'auto_refund';
        } else {
            return 'manual_refund';
        }
    } else {
        return 'unknown_type';
    }
}
```

### Good Code (Fix)
```php
function processPayment(array $payment, array $config): string
{
    if (empty($payment)) {
        return 'invalid';
    }

    $handlers = [
        'subscription' => fn() => processSubscription($payment, $config),
        'one_time'     => fn() => processOneTime($payment),
        'refund'       => fn() => processRefund($payment),
    ];

    $handler = $handlers[$payment['type']] ?? null;
    return $handler ? $handler() : 'unknown_type';
}

function processSubscription(array $payment, array $config): string
{
    if ($payment['amount'] > 10000 || $config['force_review']) {
        return classifySubscriptionReview($payment);
    }
    if ($payment['amount'] > 5000 && $payment['recurring']) {
        return 'recurring_review';
    }
    return 'auto_approve';
}

function classifySubscriptionReview(array $payment): string
{
    if ($payment['currency'] !== 'USD' && $payment['currency'] !== 'EUR') {
        return 'fx_review';
    }
    return $payment['customer']['is_vip'] ? 'vip_review' : 'standard_review';
}

function processOneTime(array $payment): string
{
    return match ($payment['method']) {
        'credit_card'   => ($payment['amount'] > 1000 && !$payment['verified']) ? 'fraud_check' : 'process_card',
        'bank_transfer' => $payment['domestic'] ? 'process_domestic' : 'process_international',
        'paypal'        => 'process_paypal',
        default         => 'unsupported_method',
    };
}

function processRefund(array $payment): string
{
    if ($payment['original_settled'] && $payment['amount'] <= $payment['original_amount']) {
        return 'auto_refund';
    }
    return 'manual_refund';
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_if_clause`, `switch_case` (case), `default_case` (default), `for_statement`, `foreach_statement`, `while_statement`, `do_statement`, `binary_expression` (with `&&`, `||`, `and`, `or`), `conditional_expression` (`?:`), `catch_clause`
- **Detection approach**: Count decision points within a function body. Each `if`, `elseif`, `case`, `default`, `for`, `foreach`, `while`, `do-while`, `&&`, `||`, `and`, `or`, `?:`, and `catch` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_definition body: (compound_statement) @fn_body) @fn
(method_declaration body: (compound_statement) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(else_if_clause) @decision
(switch_case) @decision
(default_case) @decision
(for_statement) @decision
(foreach_statement) @decision
(while_statement) @decision
(do_statement) @decision
(conditional_expression) @decision
(catch_clause) @decision
(binary_expression operator: ["&&" "||" "and" "or"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/elseif/else or switch statements that compound complexity. PHP's loose typing can tempt developers into adding extra validation checks that deepen nesting.

### Bad Code (Anti-pattern)
```php
function handleUpload(array $file, array $user, array $config): array
{
    if (!empty($file)) {
        if ($file['error'] === UPLOAD_ERR_OK) {
            if ($file['size'] <= $config['max_size']) {
                $ext = pathinfo($file['name'], PATHINFO_EXTENSION);
                if (in_array($ext, $config['allowed_extensions'])) {
                    if ($user['is_authenticated']) {
                        if ($user['storage_used'] + $file['size'] <= $user['storage_limit']) {
                            $path = saveFile($file, $user);
                            return ['success' => true, 'path' => $path];
                        } else {
                            return ['success' => false, 'error' => 'storage_limit_exceeded'];
                        }
                    } else {
                        return ['success' => false, 'error' => 'not_authenticated'];
                    }
                } else {
                    return ['success' => false, 'error' => 'invalid_extension'];
                }
            } else {
                return ['success' => false, 'error' => 'file_too_large'];
            }
        } else {
            return ['success' => false, 'error' => 'upload_error'];
        }
    } else {
        return ['success' => false, 'error' => 'no_file'];
    }
}
```

### Good Code (Fix)
```php
function handleUpload(array $file, array $user, array $config): array
{
    if (empty($file)) {
        return failure('no_file');
    }
    if ($file['error'] !== UPLOAD_ERR_OK) {
        return failure('upload_error');
    }
    if ($file['size'] > $config['max_size']) {
        return failure('file_too_large');
    }

    $ext = pathinfo($file['name'], PATHINFO_EXTENSION);
    if (!in_array($ext, $config['allowed_extensions'])) {
        return failure('invalid_extension');
    }
    if (!$user['is_authenticated']) {
        return failure('not_authenticated');
    }
    if ($user['storage_used'] + $file['size'] > $user['storage_limit']) {
        return failure('storage_limit_exceeded');
    }

    $path = saveFile($file, $user);
    return ['success' => true, 'path' => $path];
}

function failure(string $reason): array
{
    return ['success' => false, 'error' => $reason];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `compound_statement` body
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function boundary. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested if statements (3+ levels)
(if_statement
  body: (compound_statement
    (if_statement
      body: (compound_statement
        (if_statement) @deeply_nested))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
