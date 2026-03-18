# Memory Leak Indicators -- PHP

## Overview
Memory leaks in PHP are less impactful in traditional PHP-FPM (process-per-request) but critical in long-running processes like queue workers, daemons, ReactPHP servers, or Swoole applications where memory accumulates across requests.

## Why It's a Scalability Concern
Long-running PHP workers and async servers don't get the benefit of per-request memory cleanup. Arrays that grow without bounds, unclosed file handles, and static property accumulation persist across the worker's lifetime. Celery-style workers processing millions of jobs will eventually hit PHP's `memory_limit` and crash.

## Applicability
- **Relevance**: medium
- **Languages covered**: .php
- **Frameworks/libraries**: Laravel (queue workers), Symfony Messenger, ReactPHP, Swoole

---

## Pattern 1: Array Append in Loop Without Bounds

### Description
Using `$array[] =` or `array_push()` inside a loop without any size limit or pruning, causing unbounded array growth.

### Bad Code (Anti-pattern)
```php
class QueueWorker
{
    private array $processedJobs = [];

    public function handle(Job $job): void
    {
        $result = $this->process($job);
        $this->processedJobs[] = $result; // grows forever in long-running worker
    }
}
```

### Good Code (Fix)
```php
class QueueWorker
{
    private array $processedJobs = [];
    private const MAX_LOG_SIZE = 1000;

    public function handle(Job $job): void
    {
        $result = $this->process($job);
        $this->processedJobs[] = $result;
        if (count($this->processedJobs) > self::MAX_LOG_SIZE) {
            $this->processedJobs = array_slice($this->processedJobs, -self::MAX_LOG_SIZE);
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `subscript_expression`, `function_call_expression`, `for_statement`, `foreach_statement`
- **Detection approach**: Find `assignment_expression` where the left side is a `subscript_expression` with empty index (`$arr[] = ...`) inside a loop or a method that's likely called repeatedly. Also find `function_call_expression` calling `array_push` inside loops.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (subscript_expression
    (variable_name) @array))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `array_growth_no_bounds`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: fopen() Without fclose()

### Description
Opening a file with `fopen()` without a corresponding `fclose()` in the same function, risking file descriptor leaks in long-running processes.

### Bad Code (Anti-pattern)
```php
function appendLog(string $message): void
{
    $fp = fopen('/var/log/app.log', 'a');
    fwrite($fp, $message . "\n");
    // fclose not called — file descriptor leaks
}
```

### Good Code (Fix)
```php
function appendLog(string $message): void
{
    $fp = fopen('/var/log/app.log', 'a');
    try {
        fwrite($fp, $message . "\n");
    } finally {
        fclose($fp);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `assignment_expression`
- **Detection approach**: Find `function_call_expression` calling `fopen` assigned to a variable. Search the same function scope for `fclose` called with that variable. Flag if no `fclose` exists.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (variable_name) @handle
  right: (function_call_expression
    function: (name) @func
    (#eq? @func "fopen")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `fopen_without_fclose`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Static Property Accumulation

### Description
A `static` class property that only grows (via array append or addition) with no pruning, persisting across all requests in long-running processes.

### Bad Code (Anti-pattern)
```php
class EventLogger
{
    private static array $events = [];

    public static function log(string $event): void
    {
        self::$events[] = ['event' => $event, 'time' => microtime(true)];
    }
}
```

### Good Code (Fix)
```php
class EventLogger
{
    private static array $events = [];
    private const MAX_EVENTS = 5000;

    public static function log(string $event): void
    {
        self::$events[] = ['event' => $event, 'time' => microtime(true)];
        if (count(self::$events) > self::MAX_EVENTS) {
            self::$events = array_slice(self::$events, -self::MAX_EVENTS);
        }
    }

    public static function flush(): void
    {
        self::$events = [];
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `property_declaration`, `static_modifier`, `assignment_expression`, `scoped_property_access_expression`
- **Detection approach**: Find `property_declaration` with `static` modifier. Search the class for `assignment_expression` where the left side is `self::$prop[]` (subscript on static property). Flag if no array_slice, unset, or reassignment to `[]` exists.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (subscript_expression
    (scoped_property_access_expression
      scope: (name) @scope
      name: (variable_name) @prop)
    (#eq? @scope "self")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `static_property_growth`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: Database Connection Not Closed

### Description
Creating PDO or mysqli connections without closing them, leaking connections in long-running processes where PHP's end-of-script cleanup doesn't apply.

### Bad Code (Anti-pattern)
```php
function fetchData(): array
{
    $pdo = new PDO('mysql:host=localhost;dbname=app', 'user', 'pass');
    $stmt = $pdo->query('SELECT * FROM data');
    return $stmt->fetchAll();
    // $pdo connection never explicitly closed
}
```

### Good Code (Fix)
```php
function fetchData(): array
{
    $pdo = new PDO('mysql:host=localhost;dbname=app', 'user', 'pass');
    try {
        $stmt = $pdo->query('SELECT * FROM data');
        return $stmt->fetchAll();
    } finally {
        $pdo = null; // explicitly close PDO connection
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `name`, `assignment_expression`
- **Detection approach**: Find `object_creation_expression` creating `PDO` or `mysqli` assigned to a local variable. Check the function for an explicit `$var = null` or `mysqli_close($var)` call. Flag if no cleanup exists.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (variable_name) @conn
  right: (object_creation_expression
    (name) @class
    (#match? @class "^(PDO|mysqli)$")))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `db_connection_not_closed`
- **Severity**: warning
- **Confidence**: medium
