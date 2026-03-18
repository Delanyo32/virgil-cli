# Resource Lifecycle -- PHP

## Overview
Resources that are acquired but never properly released cause file descriptor leaks and database connection exhaustion. In PHP, the most common manifestations are file handles opened with `fopen` without a matching `fclose`, and PDO connections that are not cleaned up in long-running workers.

## Why It's a Tech Debt Concern
Traditional PHP request-lifecycle applications mask resource leaks because all resources are released when the process exits at the end of each request. However, modern PHP increasingly runs in long-lived contexts: queue workers (Laravel Horizon, Symfony Messenger), Swoole/RoadRunner servers, ReactPHP event loops, and CLI daemons. In these contexts, leaked file handles accumulate until the OS limit is hit ("Too many open files"), and leaked database connections exhaust the pool, causing the entire worker to stall. Even in traditional request-lifecycle PHP, leaked handles within a single request can cause issues when processing large batches of files.

## Applicability
- **Relevance**: high (file I/O, database access, long-running workers)
- **Languages covered**: `.php`
- **Frameworks/libraries**: Laravel (queue workers, Horizon), Symfony (Messenger), Swoole, RoadRunner, ReactPHP

---

## Pattern 1: fopen Without Matching fclose

### Description
Opening a file with `fopen()` and assigning the handle to a variable, but failing to call `fclose()` on all code paths. If an exception or early return occurs between `fopen` and `fclose`, the file handle leaks. PHP has no equivalent of Python's `with` or C#'s `using` for automatic cleanup, so developers must manually ensure `fclose` is called in `finally` blocks or before every return.

### Bad Code (Anti-pattern)
```php
// fclose never reached if fwrite throws or early return
function writeLog(string $path, string $message): void
{
    $handle = fopen($path, 'a');
    if ($handle === false) {
        throw new RuntimeException("Cannot open $path");
    }
    fwrite($handle, date('Y-m-d H:i:s') . " $message\n");
    // If fwrite fails or an exception is thrown above, handle leaks
    fclose($handle);
}

// Multiple handles opened, inner not closed on error
function mergeFiles(array $inputPaths, string $outputPath): void
{
    $out = fopen($outputPath, 'w');
    foreach ($inputPaths as $path) {
        $in = fopen($path, 'r');
        while (!feof($in)) {
            fwrite($out, fread($in, 8192));
        }
        fclose($in);
    }
    fclose($out);
    // If fread throws, both $in and $out leak
}

// Handle returned by fopen but never closed
function countLines(string $path): int
{
    $handle = fopen($path, 'r');
    $count = 0;
    while (fgets($handle) !== false) {
        $count++;
    }
    return $count; // fclose never called
}
```

### Good Code (Fix)
```php
// try/finally ensures fclose on all paths
function writeLog(string $path, string $message): void
{
    $handle = fopen($path, 'a');
    if ($handle === false) {
        throw new RuntimeException("Cannot open $path");
    }
    try {
        fwrite($handle, date('Y-m-d H:i:s') . " $message\n");
    } finally {
        fclose($handle);
    }
}

// Nested try/finally for multiple handles
function mergeFiles(array $inputPaths, string $outputPath): void
{
    $out = fopen($outputPath, 'w');
    if ($out === false) {
        throw new RuntimeException("Cannot open $outputPath");
    }
    try {
        foreach ($inputPaths as $path) {
            $in = fopen($path, 'r');
            if ($in === false) {
                throw new RuntimeException("Cannot open $path");
            }
            try {
                while (!feof($in)) {
                    fwrite($out, fread($in, 8192));
                }
            } finally {
                fclose($in);
            }
        }
    } finally {
        fclose($out);
    }
}

// Use file_get_contents for simple read operations
function countLines(string $path): int
{
    $content = file_get_contents($path);
    if ($content === false) {
        throw new RuntimeException("Cannot read $path");
    }
    return substr_count($content, "\n");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `function_call_expression`, `name`, `try_statement`, `finally_clause`
- **Detection approach**: Find `function_call_expression` nodes calling `fopen` whose return value is assigned to a variable. Check if the enclosing function body contains a matching `fclose` call with the same variable, and whether that `fclose` is inside a `finally_clause` of a `try_statement`. Flag `fopen` assignments where `fclose` is called only in the happy path without `finally` protection.
- **S-expression query sketch**:
  ```scheme
  ;; fopen assigned to variable
  (assignment_expression
    left: (variable_name) @handle_var
    right: (function_call_expression
      function: (name) @func_name))

  ;; fclose in finally (safe)
  (finally_clause
    body: (compound_statement
      (expression_statement
        (function_call_expression
          function: (name) @close_func
          arguments: (arguments
            (variable_name) @handle_ref)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `file_handle_leak`
- **Pattern name**: `fopen_without_fclose`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: PDO Connection Not Cleaned Up in Long-Running Workers

### Description
Creating a PDO database connection in a long-running worker (queue consumer, daemon, Swoole handler) without proper lifecycle management. Connections that are held open indefinitely can become stale (server-side timeout), and connections created per-job without cleanup exhaust the database's connection limit. PHP's garbage collector may not run frequently enough in long-lived processes to finalize PDO objects.

### Bad Code (Anti-pattern)
```php
// Connection created per job, never explicitly closed
class QueueWorker
{
    public function handle(Job $job): void
    {
        $pdo = new PDO('mysql:host=localhost;dbname=app', 'user', 'pass');
        $stmt = $pdo->prepare('INSERT INTO results (job_id, data) VALUES (?, ?)');
        $stmt->execute([$job->id, $job->result]);
        // $pdo goes out of scope but GC may not collect it immediately
        // In a tight loop, connections pile up
    }
}

// Connection stored as property, never reconnected after timeout
class ReportGenerator
{
    private PDO $db;

    public function __construct(string $dsn)
    {
        $this->db = new PDO($dsn);
    }

    // Runs for hours -- connection may time out
    public function generateAll(array $reports): void
    {
        foreach ($reports as $report) {
            // MySQL server has gone away -- stale connection
            $this->db->query("SELECT * FROM data WHERE report_id = {$report->id}");
        }
    }
}

// Swoole handler creating connections per request
$server->on('request', function ($request, $response) {
    $db = new PDO('mysql:host=localhost;dbname=app', 'user', 'pass');
    $data = $db->query('SELECT * FROM items')->fetchAll();
    $response->end(json_encode($data));
    // Connection leaked per request in long-lived Swoole process
});
```

### Good Code (Fix)
```php
// Connection pool / factory with explicit lifecycle
class QueueWorker
{
    private PDO $pdo;

    public function __construct(string $dsn, string $user, string $pass)
    {
        $this->pdo = new PDO($dsn, $user, $pass, [
            PDO::ATTR_PERSISTENT => true,
            PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        ]);
    }

    public function handle(Job $job): void
    {
        $stmt = $this->pdo->prepare('INSERT INTO results (job_id, data) VALUES (?, ?)');
        $stmt->execute([$job->id, $job->result]);
    }

    public function __destruct()
    {
        $this->pdo = null; // Explicitly close connection
    }
}

// Reconnect-on-failure pattern for long-running processes
class ReportGenerator
{
    private ?PDO $db = null;
    private string $dsn;

    public function __construct(string $dsn)
    {
        $this->dsn = $dsn;
    }

    private function getConnection(): PDO
    {
        if ($this->db === null) {
            $this->db = new PDO($this->dsn);
            $this->db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
        }
        return $this->db;
    }

    private function reconnect(): PDO
    {
        $this->db = null;
        return $this->getConnection();
    }

    public function generateAll(array $reports): void
    {
        foreach ($reports as $report) {
            try {
                $this->getConnection()->query(
                    "SELECT * FROM data WHERE report_id = " . (int)$report->id
                );
            } catch (PDOException $e) {
                if ($this->isConnectionLost($e)) {
                    $this->reconnect()->query(
                        "SELECT * FROM data WHERE report_id = " . (int)$report->id
                    );
                } else {
                    throw $e;
                }
            }
        }
    }

    private function isConnectionLost(PDOException $e): bool
    {
        return str_contains($e->getMessage(), 'server has gone away')
            || str_contains($e->getMessage(), 'Lost connection');
    }
}

// Swoole with connection pool
$pool = new ConnectionPool(function () {
    return new PDO('mysql:host=localhost;dbname=app', 'user', 'pass');
}, maxSize: 10);

$server->on('request', function ($request, $response) use ($pool) {
    $db = $pool->get();
    try {
        $data = $db->query('SELECT * FROM items')->fetchAll();
        $response->end(json_encode($data));
    } finally {
        $pool->put($db);
    }
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `named_type`, `method_declaration`, `class_declaration`
- **Detection approach**: Find `object_creation_expression` nodes creating `PDO` instances inside method bodies. Check if the method belongs to a class that appears to be a worker, handler, or daemon (heuristic: class name contains `Worker`, `Handler`, `Consumer`, `Daemon`, or method is inside a closure passed to `->on()`). Flag PDO creation inside loop bodies or handler methods where the instance is assigned to a local variable rather than a class property with lifecycle management.
- **S-expression query sketch**:
  ```scheme
  ;; PDO created in a method body
  (method_declaration
    name: (name) @method_name
    body: (compound_statement
      (expression_statement
        (assignment_expression
          left: (variable_name) @var_name
          right: (object_creation_expression
            (name) @class_name)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `file_handle_leak`
- **Pattern name**: `pdo_connection_leak_in_worker`
- **Severity**: warning
- **Confidence**: medium
