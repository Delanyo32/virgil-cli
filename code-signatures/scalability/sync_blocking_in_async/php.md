# Sync Blocking in Async -- PHP

## Overview
PHP is traditionally synchronous, but modern frameworks like ReactPHP, Amp, and Swoole introduce event-loop-based concurrency. Blocking calls like `file_get_contents()`, `sleep()`, or `curl_exec()` inside these async contexts stall the event loop.

## Why It's a Scalability Concern
In event-loop frameworks, the loop processes all connections on a single thread. A blocking `sleep(5)` or synchronous `curl_exec()` inside a request handler freezes all other connections for the duration. This is less relevant for traditional PHP-FPM deployments but critical for long-running async PHP servers.

## Applicability
- **Relevance**: low
- **Languages covered**: .php
- **Frameworks/libraries**: ReactPHP, Amp, Swoole, PHP 8.1+ Fibers

---

## Pattern 1: Blocking I/O in Event Loop Callback

### Description
Using `file_get_contents()`, `file_put_contents()`, or `sleep()` inside a ReactPHP/Amp/Swoole callback or promise handler.

### Bad Code (Anti-pattern)
```php
$server->on('request', function ($request, $response) {
    $data = file_get_contents('/var/data/config.json'); // blocks event loop
    sleep(1); // blocks event loop
    $response->end($data);
});
```

### Good Code (Fix)
```php
$server->on('request', function ($request, $response) use ($filesystem) {
    $filesystem->file('/var/data/config.json')->getContents()
        ->then(function ($data) use ($response) {
            $response->end($data);
        });
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `anonymous_function_creation_expression`, `member_call_expression`
- **Detection approach**: Find `function_call_expression` calling `file_get_contents`, `file_put_contents`, `sleep`, `usleep` inside an `anonymous_function_creation_expression` (closure) that is an argument to methods like `->on()`, `->then()`, `->map()` on event-loop or promise objects.
- **S-expression query sketch**:
```scheme
(anonymous_function_creation_expression
  body: (compound_statement
    (expression_statement
      (function_call_expression
        function: (name) @func_name))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `blocking_io_in_event_callback`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: curl_exec() in Fiber (PHP 8.1+)

### Description
Using synchronous `curl_exec()` inside a PHP 8.1 Fiber, which blocks the fiber's execution thread. Fibers enable cooperative multitasking, but only if I/O is non-blocking.

### Bad Code (Anti-pattern)
```php
$fiber = new Fiber(function () {
    $ch = curl_init('https://api.example.com/data');
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    $result = curl_exec($ch); // blocks the fiber
    curl_close($ch);
    Fiber::suspend($result);
});
```

### Good Code (Fix)
```php
$fiber = new Fiber(function () use ($httpClient) {
    $promise = $httpClient->getAsync('https://api.example.com/data');
    Fiber::suspend($promise);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `name`, `function_call_expression`, `anonymous_function_creation_expression`
- **Detection approach**: Find `function_call_expression` calling `curl_exec` inside an `anonymous_function_creation_expression` that is an argument to `new Fiber(...)`.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  (name) @class_name
  (arguments
    (anonymous_function_creation_expression
      body: (compound_statement
        (expression_statement
          (function_call_expression
            function: (name) @func_name))))))
```

### Pipeline Mapping
- **Pipeline name**: `sync_blocking_in_async`
- **Pattern name**: `curl_exec_in_fiber`
- **Severity**: warning
- **Confidence**: medium
