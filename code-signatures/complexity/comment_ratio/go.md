# Comment Ratio -- Go

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .go
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```go
func SyncWorkers(pool *WorkerPool, tasks []Task, maxRetries int) ([]Result, error) {
	results := make([]Result, 0, len(tasks))
	errCh := make(chan error, len(tasks))
	resultCh := make(chan Result, len(tasks))

	for _, task := range tasks {
		worker := pool.Acquire()
		if worker == nil {
			for i := 0; i < maxRetries; i++ {
				time.Sleep(time.Duration(i*100) * time.Millisecond)
				worker = pool.Acquire()
				if worker != nil {
					break
				}
			}
			if worker == nil {
				errCh <- fmt.Errorf("no worker available for task %s", task.ID)
				continue
			}
		}

		go func(w *Worker, t Task) {
			defer pool.Release(w)
			res, err := w.Execute(t)
			if err != nil {
				for retry := 0; retry < maxRetries; retry++ {
					res, err = w.Execute(t)
					if err == nil {
						break
					}
					if t.Idempotent == false {
						errCh <- fmt.Errorf("non-idempotent task %s failed: %w", t.ID, err)
						return
					}
				}
				if err != nil {
					errCh <- err
					return
				}
			}
			resultCh <- res
		}(worker, task)
	}

	close(errCh)
	for err := range errCh {
		if err != nil {
			return nil, err
		}
	}
	close(resultCh)
	for res := range resultCh {
		results = append(results, res)
	}
	return results, nil
}
```

### Good Code (Fix)
```go
// SyncWorkers dispatches tasks across the pool with exponential backoff on
// worker acquisition, and per-task retry for idempotent operations.
func SyncWorkers(pool *WorkerPool, tasks []Task, maxRetries int) ([]Result, error) {
	results := make([]Result, 0, len(tasks))
	errCh := make(chan error, len(tasks))
	resultCh := make(chan Result, len(tasks))

	for _, task := range tasks {
		worker := pool.Acquire()
		if worker == nil {
			// Back off with linear delay -- pool contention is transient
			// under normal load; persistent failure means pool is undersized
			for i := 0; i < maxRetries; i++ {
				time.Sleep(time.Duration(i*100) * time.Millisecond)
				worker = pool.Acquire()
				if worker != nil {
					break
				}
			}
			if worker == nil {
				errCh <- fmt.Errorf("no worker available for task %s", task.ID)
				continue
			}
		}

		go func(w *Worker, t Task) {
			defer pool.Release(w)
			res, err := w.Execute(t)
			if err != nil {
				for retry := 0; retry < maxRetries; retry++ {
					res, err = w.Execute(t)
					if err == nil {
						break
					}
					// Non-idempotent tasks must not be retried to avoid
					// duplicate side effects (payments, notifications, etc.)
					if t.Idempotent == false {
						errCh <- fmt.Errorf("non-idempotent task %s failed: %w", t.ID, err)
						return
					}
				}
				if err != nil {
					errCh <- err
					return
				}
			}
			resultCh <- res
		}(worker, task)
	}

	close(errCh)
	for err := range errCh {
		if err != nil {
			return nil, err
		}
	}
	close(resultCh)
	for res := range resultCh {
		results = append(results, res)
	}
	return results, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration` for function bodies; `comment` for `//` and `/* */` comments
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold. Godoc convention: exported functions should have a comment starting with the function name.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_declaration
    body: (block) @function.body)

  (method_declaration
    body: (block) @function.body)

  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```go
func UpdateConfig(cfg *Config, key string, value string) error {
	// Lock the mutex
	cfg.mu.Lock()
	// Defer unlock
	defer cfg.mu.Unlock()

	// Check if key is empty
	if key == "" {
		// Return error
		return fmt.Errorf("key cannot be empty")
	}

	// Set the value in the map
	cfg.values[key] = value

	// Increment the version
	cfg.version++

	// Return nil
	return nil
}
```

### Good Code (Fix)
```go
func UpdateConfig(cfg *Config, key string, value string) error {
	cfg.mu.Lock()
	defer cfg.mu.Unlock()

	if key == "" {
		return fmt.Errorf("key cannot be empty")
	}

	cfg.values[key] = value

	// Version bump triggers watchers to reload their cached config
	cfg.version++

	return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `short_var_declaration`, `assignment_statement`, `return_statement`, `if_statement`, `defer_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (block
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
