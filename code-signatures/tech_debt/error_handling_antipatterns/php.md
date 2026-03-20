# Error Handling Anti-patterns -- PHP

## Overview
Errors that are silently suppressed, broadly caught, or swallowed make debugging impossible and hide real failures. In PHP, the error suppression operator `@`, empty catch blocks, and catch blocks that only log without rethrowing are the most common anti-patterns.

## Why It's a Tech Debt Concern
The `@` operator silences all errors including warnings that indicate real problems, making it impossible to detect file permission issues, network failures, or deprecated function usage. Empty catch blocks discard exceptions entirely, allowing the application to continue with corrupted state. Catching broad `\Throwable` or `\Exception` and only logging means callers believe the operation succeeded when it did not, leading to data inconsistencies that are discovered only when users report problems.

## Applicability
- **Relevance**: high
- **Languages covered**: `.php`

---

## Pattern 1: Error Suppression Operator

### Description
Using the `@` operator before function calls to silence all PHP errors and warnings. This suppresses error reporting for that expression, hiding file not found errors, permission denied warnings, connection failures, and deprecation notices.

### Bad Code (Anti-pattern)
```php
$connection = @mysqli_connect($host, $user, $pass, $db);
if (!$connection) {
    // No error details available
    die('Connection failed');
}

$data = @file_get_contents($url);
$result = @unserialize($data);
$handle = @fopen($path, 'r');
```

### Good Code (Fix)
```php
try {
    $connection = mysqli_connect($host, $user, $pass, $db);
} catch (\mysqli_sql_exception $e) {
    $logger->error('Database connection failed', [
        'host' => $host,
        'error' => $e->getMessage(),
    ]);
    throw new DatabaseException('Could not connect to database', 0, $e);
}

$data = file_get_contents($url);
if ($data === false) {
    throw new FileException("Failed to read from {$url}");
}

$result = unserialize($data);
if ($result === false && $data !== serialize(false)) {
    throw new DeserializationException("Failed to unserialize data");
}

$handle = fopen($path, 'r');
if ($handle === false) {
    throw new FileException("Cannot open file: {$path}");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `error_suppression_expression`, `function_call_expression`
- **Detection approach**: Find `error_suppression_expression` nodes (the `@` operator applied to an expression). These are direct tree-sitter node types in the PHP grammar. Flag any occurrence, as the `@` operator is almost always an anti-pattern.
- **S-expression query sketch**:
```scheme
(error_suppression_expression
  (function_call_expression
    function: (name) @suppressed_function))
```

### Pipeline Mapping
- **Pipeline name**: `error_suppression`
- **Pattern name**: `at_operator`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Empty Catch Block

### Description
A `catch` block that contains no statements, or catches `\Exception` or `\Throwable` with an empty body. The exception is completely discarded, and execution continues as if nothing went wrong.

### Bad Code (Anti-pattern)
```php
try {
    $order = $this->orderRepository->save($orderData);
    $this->paymentService->charge($order);
    $this->emailService->sendConfirmation($order);
} catch (Exception $e) {
}

try {
    $cache->set($key, $value, $ttl);
} catch (\Throwable $e) {
    // Silence cache errors
}
```

### Good Code (Fix)
```php
try {
    $order = $this->orderRepository->save($orderData);
    $this->paymentService->charge($order);
    $this->emailService->sendConfirmation($order);
} catch (PaymentException $e) {
    $this->logger->error('Payment failed for order', [
        'order_id' => $orderData['id'],
        'error' => $e->getMessage(),
    ]);
    throw new OrderProcessingException('Payment processing failed', 0, $e);
} catch (EmailException $e) {
    $this->logger->warning('Confirmation email failed', [
        'order_id' => $order->getId(),
        'error' => $e->getMessage(),
    ]);
    // Email failure is non-critical, queue for retry
    $this->retryQueue->push('send_confirmation', $order->getId());
}

try {
    $cache->set($key, $value, $ttl);
} catch (CacheException $e) {
    $this->logger->warning('Cache write failed', [
        'key' => $key,
        'error' => $e->getMessage(),
    ]);
    // Cache failure is non-critical but should be monitored
    $this->metrics->increment('cache.write_failures');
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `compound_statement`
- **Detection approach**: Find `catch_clause` nodes whose body `compound_statement` has zero child statements. Also flag catch blocks containing only comments. Check the `type_list` in the catch clause to identify whether broad types like `Exception` or `Throwable` are caught.
- **S-expression query sketch**:
```scheme
(try_statement
  (catch_clause
    type: (type_list
      (named_type
        (name) @exception_type))
    (compound_statement) @catch_body))
```

### Pipeline Mapping
- **Pipeline name**: `silent_exception`
- **Pattern name**: `empty_catch_block`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Silent Exception

### Description
A catch block that logs the exception (via `error_log`, `$logger->error()`, etc.) but does not rethrow, return an error value, or take any corrective action. The exception is acknowledged but swallowed, and the caller assumes the operation succeeded.

### Bad Code (Anti-pattern)
```php
public function importUsers(string $csvPath): array
{
    $users = [];
    try {
        $handle = fopen($csvPath, 'r');
        while ($row = fgetcsv($handle)) {
            $users[] = $this->createUser($row);
        }
        fclose($handle);
    } catch (\Exception $e) {
        error_log('Import failed: ' . $e->getMessage());
    }
    return $users; // Returns partial results on failure
}

public function processPayment(Order $order): void
{
    try {
        $this->gateway->charge($order->getTotal(), $order->getPaymentMethod());
        $order->setStatus('paid');
    } catch (PaymentException $e) {
        $this->logger->error('Payment failed', ['error' => $e->getMessage()]);
        // Order status remains unchanged, caller doesn't know payment failed
    }
}
```

### Good Code (Fix)
```php
public function importUsers(string $csvPath): array
{
    $handle = fopen($csvPath, 'r');
    if ($handle === false) {
        throw new ImportException("Cannot open CSV file: {$csvPath}");
    }

    try {
        $users = [];
        while ($row = fgetcsv($handle)) {
            $users[] = $this->createUser($row);
        }
        return $users;
    } catch (\Exception $e) {
        throw new ImportException("User import failed at row " . count($users), 0, $e);
    } finally {
        fclose($handle);
    }
}

public function processPayment(Order $order): void
{
    try {
        $this->gateway->charge($order->getTotal(), $order->getPaymentMethod());
        $order->setStatus('paid');
    } catch (PaymentException $e) {
        $this->logger->error('Payment failed', [
            'order_id' => $order->getId(),
            'amount' => $order->getTotal(),
            'error' => $e->getMessage(),
        ]);
        $order->setStatus('payment_failed');
        throw new OrderException('Payment processing failed for order ' . $order->getId(), 0, $e);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `catch_clause`, `compound_statement`, `expression_statement`, `function_call_expression`, `member_call_expression`
- **Detection approach**: Find `catch_clause` bodies that contain a `function_call_expression` or `member_call_expression` targeting logging functions (`error_log`, `$this->logger->error`, `$logger->warning`, etc.) but do not contain a `throw_statement`. Check that no `throw_expression` exists as a descendant of the catch body.
- **S-expression query sketch**:
```scheme
(catch_clause
  body: (compound_statement
    (expression_statement
      (member_call_expression
        object: (member_access_expression
          name: (name) @logger_var)
        name: (name) @log_method))))
```

### Pipeline Mapping
- **Pipeline name**: `silent_exception`
- **Pattern name**: `logged_not_rethrown`
- **Severity**: warning
- **Confidence**: medium
