# Concurrency Misuse -- PHP

## Overview
Traditional PHP runs in a share-nothing, request-per-process model where concurrency issues are rare. However, race conditions arise in file-based session handling under concurrent requests, and the growing adoption of long-running async runtimes (Swoole, ReactPHP, Amp) introduces shared mutable state problems that PHP developers are often unprepared for.

## Why It's a Tech Debt Concern
File-based session race conditions cause lost updates when concurrent AJAX requests modify the same session — the last request to write wins, silently discarding changes from other requests. In Swoole/ReactPHP workers, shared state across coroutines or event loop callbacks leads to data corruption that is extremely difficult to debug because the traditional "each request is isolated" mental model no longer applies.

## Applicability
- **Relevance**: low
- **Languages covered**: `.php`
- **Frameworks/libraries**: PHP sessions (file handler), Swoole, ReactPHP, Amp, Laravel Octane

---

## Pattern 1: Race Condition in File-Based Sessions

### Description
Using the default file-based session handler (`session_start()`) in applications that receive concurrent requests for the same session (e.g., AJAX-heavy SPAs). PHP's file session handler locks the session file for the duration of the request, causing concurrent requests to serialize. When `session_write_close()` is called early or sessions are read-then-written without holding the lock, updates from concurrent requests are lost.

### Bad Code (Anti-pattern)
```php
class CartController
{
    public function addItem(Request $request): Response
    {
        session_start();
        session_write_close();  // Releases the lock immediately

        // Slow operation happens after lock release
        $product = $this->productService->find($request->get('product_id'));
        $price = $this->pricingService->calculate($product);

        // Re-opens session — but another request may have modified it
        session_start();
        $_SESSION['cart'][] = ['product' => $product->id, 'price' => $price];
        session_write_close();

        return new Response('Added');
    }

    public function updateQuantity(Request $request): Response
    {
        session_start();
        $cart = $_SESSION['cart'];  // Read
        session_write_close();     // Release lock

        // Modify in memory while another request may be modifying the same session
        $cart[$request->get('index')]['quantity'] = $request->get('quantity');

        session_start();
        $_SESSION['cart'] = $cart;  // Write — overwrites any concurrent changes
        session_write_close();

        return new Response('Updated');
    }
}
```

### Good Code (Fix)
```php
class CartController
{
    public function addItem(Request $request): Response
    {
        // Pre-compute everything before touching the session
        $product = $this->productService->find($request->get('product_id'));
        $price = $this->pricingService->calculate($product);

        // Hold the session lock for the minimum required duration
        session_start();
        $_SESSION['cart'][] = ['product' => $product->id, 'price' => $price];
        session_write_close();

        return new Response('Added');
    }

    public function updateQuantity(Request $request): Response
    {
        session_start();
        $_SESSION['cart'][$request->get('index')]['quantity'] = $request->get('quantity');
        session_write_close();

        return new Response('Updated');
    }
}

// Or better: use a database/Redis session handler
// ini_set('session.save_handler', 'redis');
// ini_set('session.save_path', 'tcp://127.0.0.1:6379');
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name` (function name), `member_access_expression`
- **Detection approach**: Find functions or methods that call `session_start()` more than once. Also flag patterns where `session_write_close()` is followed by additional code that modifies `$_SESSION` after a second `session_start()`. Count `session_start()` invocations within a single function body.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name)
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `session_race_condition`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Shared State in Swoole/ReactPHP Workers

### Description
In Swoole or ReactPHP long-running processes, class properties or global variables persist across requests/coroutines. Modifying shared state without synchronization (Swoole channels, locks, or atomic operations) leads to data corruption when multiple coroutines or event loop callbacks access the same data concurrently.

### Bad Code (Anti-pattern)
```php
class RequestHandler
{
    private array $requestCount = [];
    private array $cache = [];

    public function handle(SwooleHttpRequest $request, SwooleHttpResponse $response): void
    {
        $path = $request->server['request_uri'];

        // Shared mutable state — multiple coroutines modify concurrently
        if (!isset($this->requestCount[$path])) {
            $this->requestCount[$path] = 0;
        }
        $this->requestCount[$path]++;  // Race condition: read-modify-write

        // Shared cache without synchronization
        if (!isset($this->cache[$path])) {
            // Multiple coroutines may compute this simultaneously
            $this->cache[$path] = $this->expensiveCompute($path);
        }

        $response->end(json_encode(['count' => $this->requestCount[$path]]));
    }
}

$server = new SwooleHttpServer('0.0.0.0', 9501);
$handler = new RequestHandler();
$server->on('request', [$handler, 'handle']);
$server->start();
```

### Good Code (Fix)
```php
use Swoole\Table;
use Swoole\Atomic;

class RequestHandler
{
    private Table $requestCounts;
    private Table $cache;

    public function __construct()
    {
        // Swoole Table provides process-safe shared memory
        $this->requestCounts = new Table(1024);
        $this->requestCounts->column('count', Table::TYPE_INT);
        $this->requestCounts->create();

        $this->cache = new Table(1024);
        $this->cache->column('value', Table::TYPE_STRING, 4096);
        $this->cache->create();
    }

    public function handle(SwooleHttpRequest $request, SwooleHttpResponse $response): void
    {
        $path = $request->server['request_uri'];

        // Atomic increment via Swoole Table
        $this->requestCounts->incr($path, 'count', 1);

        if (!$this->cache->exists($path)) {
            $result = $this->expensiveCompute($path);
            $this->cache->set($path, ['value' => json_encode($result)]);
        }

        $count = $this->requestCounts->get($path, 'count');
        $response->end(json_encode(['count' => $count]));
    }
}

$server = new SwooleHttpServer('0.0.0.0', 9501);
$handler = new RequestHandler();
$server->on('request', [$handler, 'handle']);
$server->start();
```

### Tree-sitter Detection Strategy
- **Target node types**: `property_declaration`, `member_access_expression`, `assignment_expression`, `augmented_assignment_expression`, `class_declaration`
- **Detection approach**: Find classes that are instantiated and passed to `$server->on('request', ...)` or used as ReactPHP/Amp request handlers. Within those classes, find mutable instance properties (arrays, objects) that are both read and written inside the request handler method. Flag when no Swoole synchronization primitives (`Table`, `Atomic`, `Lock`, `Channel`) are used.
- **S-expression query sketch**:
```scheme
(class_declaration
  body: (declaration_list
    (property_declaration
      (property_element
        (variable_name) @prop_name))
    (method_declaration
      name: (name) @method_name
      body: (compound_statement
        (expression_statement
          (augmented_assignment_expression
            left: (member_access_expression
              name: (variable_name) @accessed_prop)))))))
```

### Pipeline Mapping
- **Pipeline name**: `concurrency_misuse`
- **Pattern name**: `shared_state_in_worker`
- **Severity**: warning
- **Confidence**: medium
