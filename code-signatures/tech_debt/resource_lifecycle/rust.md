# Resource Lifecycle -- Rust

## Overview
Rust's ownership system and RAII prevent most resource leaks at compile time, but certain patterns can still cause logical resource mismanagement. The most common issues are holding a `MutexGuard` across an `.await` point (causing deadlocks in async code) and forgetting to drop large allocations in long-lived scopes (causing unnecessary memory pressure).

## Why It's a Tech Debt Concern
Holding a `MutexGuard` across an `.await` point means the lock is held while the task is suspended, blocking all other tasks that need the same mutex. Since async runtimes multiplex tasks on a limited thread pool, this can deadlock the entire runtime. Large allocations kept alive in long-lived scopes (event loops, server handlers, daemon threads) prevent the allocator from reclaiming memory, leading to steadily increasing RSS that mimics a memory leak even though the values are technically reachable.

## Applicability
- **Relevance**: high (async Rust and long-running services)
- **Languages covered**: `.rs`
- **Frameworks/libraries**: tokio, async-std, any async runtime using `std::sync::Mutex`

---

## Pattern 1: Holding MutexGuard Across Await Point

### Description
Locking a `Mutex` (either `std::sync::Mutex` or `parking_lot::Mutex`) and holding the returned `MutexGuard` across an `.await` expression. The guard keeps the lock held while the task is suspended at the await point, preventing other tasks from acquiring the lock. With `std::sync::Mutex`, this causes deadlocks; with `tokio::sync::Mutex`, it causes unnecessary contention.

### Bad Code (Anti-pattern)
```rust
async fn update_cache(cache: Arc<Mutex<HashMap<String, String>>>, key: String) {
    let mut guard = cache.lock().unwrap();
    // Guard is held across this await point
    let value = fetch_from_remote(&key).await;
    guard.insert(key, value);
}

async fn process_batch(state: Arc<Mutex<AppState>>) {
    let state = state.lock().unwrap();
    for item in &state.pending_items {
        // Lock held across every iteration's await
        send_notification(item).await;
    }
}

async fn log_and_update(data: Arc<Mutex<Vec<Entry>>>) {
    let mut entries = data.lock().unwrap();
    entries.push(Entry::new("start"));
    do_async_work().await;  // Lock held here
    entries.push(Entry::new("end"));
}
```

### Good Code (Fix)
```rust
async fn update_cache(cache: Arc<Mutex<HashMap<String, String>>>, key: String) {
    // Fetch first, then lock briefly to insert
    let value = fetch_from_remote(&key).await;
    let mut guard = cache.lock().unwrap();
    guard.insert(key, value);
}

async fn process_batch(state: Arc<Mutex<AppState>>) {
    // Clone data out while holding the lock briefly
    let items = {
        let state = state.lock().unwrap();
        state.pending_items.clone()
    };
    for item in &items {
        send_notification(item).await;
    }
}

async fn log_and_update(data: Arc<Mutex<Vec<Entry>>>) {
    {
        let mut entries = data.lock().unwrap();
        entries.push(Entry::new("start"));
    } // Lock dropped before await
    do_async_work().await;
    {
        let mut entries = data.lock().unwrap();
        entries.push(Entry::new("end"));
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `let_declaration`, `call_expression`, `field_expression`, `await_expression`
- **Detection approach**: Find `let_declaration` nodes where the value is a `call_expression` with `.lock()` (via `field_expression` with `field_identifier` equal to `lock`). Then scan the remaining statements in the enclosing block for `await_expression` nodes that appear before the guard variable goes out of scope (i.e., before the enclosing block ends or the variable is explicitly dropped).
- **S-expression query sketch**:
  ```scheme
  ;; MutexGuard binding
  (let_declaration
    pattern: (_) @guard_var
    value: (call_expression
      function: (field_expression
        field: (field_identifier) @lock_method)))

  ;; Await expression in same scope
  (await_expression
    (_) @awaited_expr)
  ```

### Pipeline Mapping
- **Pipeline name**: `mutex_guard_across_await`
- **Pattern name**: `guard_held_across_await`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Forgetting to Drop Large Allocations in Long-Lived Scopes

### Description
Creating large `Vec`, `String`, `HashMap`, or other heap-allocated structures inside a long-lived scope (event loop, server request handler loop, daemon main loop) without explicitly dropping them or scoping them to a short-lived block. The allocation persists for the lifetime of the enclosing scope, causing memory usage to grow with each iteration even though the data is no longer needed.

### Bad Code (Anti-pattern)
```rust
fn event_loop(receiver: Receiver<Event>) {
    let mut buffer = Vec::with_capacity(1024 * 1024); // 1MB
    loop {
        let event = receiver.recv().unwrap();
        buffer.clear(); // Retains allocated capacity
        process_event(&event, &mut buffer);
        // buffer holds 1MB+ permanently even when idle
    }
}

fn handle_requests(listener: TcpListener) {
    for stream in listener.incoming() {
        let mut response_body = String::with_capacity(64 * 1024);
        build_response(&stream.unwrap(), &mut response_body);
        // response_body dropped at end of loop iteration -- but large
        // temporary Vecs created inside build_response may not be
        let temp_data: Vec<u8> = read_entire_body(&stream.unwrap());
        let parsed = parse_large_payload(&temp_data);
        // temp_data is still alive here, doubling memory usage
        send_response(&stream.unwrap(), &parsed);
    }
}

async fn worker(queue: Arc<Queue>) {
    let mut cache = HashMap::new();
    loop {
        let job = queue.dequeue().await;
        let result = process_job(&job);
        cache.insert(job.id, result);
        // Cache grows unboundedly -- never pruned
    }
}
```

### Good Code (Fix)
```rust
fn event_loop(receiver: Receiver<Event>) {
    loop {
        let event = receiver.recv().unwrap();
        // Scoped allocation -- dropped each iteration
        let mut buffer = Vec::with_capacity(4096);
        process_event(&event, &mut buffer);
        // buffer dropped here, memory returned to allocator
    }
}

fn handle_requests(listener: TcpListener) {
    for stream in listener.incoming() {
        let stream = stream.unwrap();
        let parsed = {
            let temp_data: Vec<u8> = read_entire_body(&stream);
            let result = parse_large_payload(&temp_data);
            result
            // temp_data dropped here before send_response
        };
        send_response(&stream, &parsed);
    }
}

async fn worker(queue: Arc<Queue>) {
    let mut cache = HashMap::new();
    let max_cache_size = 10_000;
    loop {
        let job = queue.dequeue().await;
        let result = process_job(&job);
        cache.insert(job.id, result);
        // Prune cache to prevent unbounded growth
        if cache.len() > max_cache_size {
            let oldest: Vec<_> = cache.keys().take(max_cache_size / 2).cloned().collect();
            for key in oldest {
                cache.remove(&key);
            }
            cache.shrink_to_fit();
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `let_declaration`, `call_expression`, `loop_expression`, `for_expression`
- **Detection approach**: Find `let_declaration` nodes inside `loop_expression` or `for_expression` blocks where the value is a `call_expression` to `Vec::with_capacity`, `String::with_capacity`, `HashMap::new`, or similar large-allocation constructors. Check if the variable is defined before the loop (persisting across iterations) versus inside the loop body (scoped to iteration). Also flag `HashMap` or `Vec` variables that are inserted into within a loop but never have elements removed or the collection bounded.
- **S-expression query sketch**:
  ```scheme
  ;; Large allocation before a loop
  (let_declaration
    pattern: (_) @var_name
    value: (call_expression
      function: (scoped_identifier) @alloc_fn))

  ;; Loop that uses the variable
  (loop_expression
    body: (block
      (expression_statement
        (call_expression
          function: (field_expression
            value: (identifier) @var_ref
            field: (field_identifier) @method)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `mutex_guard_across_await`
- **Pattern name**: `undropped_large_allocation`
- **Severity**: info
- **Confidence**: medium
