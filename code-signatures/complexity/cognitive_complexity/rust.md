# Cognitive Complexity -- Rust

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, match arms, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, match expressions, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```rust
fn process_entries(entries: &[Entry], config: &Config) -> Vec<Result<Output, Error>> {
    let mut results = Vec::new();
    for entry in entries {
        if entry.is_active {
            match entry.category {
                Category::Priority => {
                    for field in &entry.fields {
                        if config.required_fields.contains(&field.name) {
                            match field.validate() {
                                Ok(val) => {
                                    if val.score < config.threshold {
                                        continue;
                                    }
                                    results.push(Ok(transform(val)));
                                }
                                Err(e) => {
                                    if config.strict {
                                        return vec![Err(e)];
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                Category::Standard => {
                    if let Some(fallback) = &entry.fallback {
                        results.push(Ok(default_transform(fallback)));
                    }
                }
                _ => {}
            }
        }
    }
    results
}
```

### Good Code (Fix)
```rust
fn process_priority_fields(
    fields: &[Field],
    config: &Config,
) -> ControlFlow<Error, Vec<Output>> {
    let mut outputs = Vec::new();
    for field in fields {
        if !config.required_fields.contains(&field.name) {
            continue;
        }
        let val = match field.validate() {
            Ok(v) => v,
            Err(e) if config.strict => return ControlFlow::Break(e),
            Err(_) => break,
        };
        if val.score >= config.threshold {
            outputs.push(transform(val));
        }
    }
    ControlFlow::Continue(outputs)
}

fn process_single_entry(entry: &Entry, config: &Config) -> Vec<Result<Output, Error>> {
    if !entry.is_active {
        return Vec::new();
    }
    match entry.category {
        Category::Priority => match process_priority_fields(&entry.fields, config) {
            ControlFlow::Continue(outputs) => outputs.into_iter().map(Ok).collect(),
            ControlFlow::Break(e) => vec![Err(e)],
        },
        Category::Standard => entry
            .fallback
            .as_ref()
            .map(|fb| vec![Ok(default_transform(fb))])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn process_entries(entries: &[Entry], config: &Config) -> Vec<Result<Output, Error>> {
    entries.iter().flat_map(|e| process_single_entry(e, config)).collect()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_expression`, `match_expression`, `for_expression`, `while_expression`, `loop_expression`, `if_let_expression`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and early `return` statements add 1 each for flow disruption. `else` clauses and `match` arms beyond the first add 1 each. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_item body: (block) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_expression) @nesting
(match_expression) @nesting
(for_expression) @nesting
(while_expression) @nesting
(loop_expression) @nesting
(if_let_expression) @nesting

;; Flow-breaking statements (add 1 each)
(continue_expression) @flow_break
(break_expression) @flow_break
(return_expression) @flow_break

;; Else clauses and match arms break linear flow
(else_clause) @flow_break
(match_arm) @flow_break
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and error handling at every step, creating a zigzag pattern of `match` on `Result`/`Option` at each operation instead of using the `?` operator or combinators, fragmenting the readable logic flow.

### Bad Code (Anti-pattern)
```rust
fn sync_user_data(user_id: &str) -> Result<SyncResult, SyncError> {
    let user = match fetch_user(user_id) {
        Ok(u) => u,
        Err(e) => return Err(SyncError::FetchFailed(e)),
    };

    let profile = match fetch_profile(&user.profile_id) {
        Ok(p) => p,
        Err(e) => return Err(SyncError::ProfileFailed(e)),
    };

    let preferences = match load_preferences(user.id) {
        Ok(p) => p,
        Err(_) => default_preferences(),
    };

    let merged = match merge_data(&user, &profile, &preferences) {
        Ok(m) => m,
        Err(e) => return Err(SyncError::MergeFailed(e)),
    };

    match save_to_cache(&merged) {
        Ok(_) => {}
        Err(e) => eprintln!("Cache save failed: {e}"),
    };

    match notify_services(&merged) {
        Ok(_) => {}
        Err(e) => return Err(SyncError::NotifyFailed(e)),
    };

    Ok(SyncResult { data: merged })
}
```

### Good Code (Fix)
```rust
fn sync_user_data(user_id: &str) -> Result<SyncResult, SyncError> {
    let user = fetch_user(user_id).map_err(SyncError::FetchFailed)?;
    let profile = fetch_profile(&user.profile_id).map_err(SyncError::ProfileFailed)?;
    let preferences = load_preferences(user.id).unwrap_or_else(|_| default_preferences());
    let merged = merge_data(&user, &profile, &preferences).map_err(SyncError::MergeFailed)?;

    if let Err(e) = save_to_cache(&merged) {
        eprintln!("Cache save failed: {e}");
    }

    notify_services(&merged).map_err(SyncError::NotifyFailed)?;

    Ok(SyncResult { data: merged })
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `match_expression`, `if_let_expression`, `macro_invocation` (for `unwrap`, `expect`)
- **Detection approach**: Count `match_expression` nodes within a single function body that match on `Result` or `Option` patterns (`Ok`/`Err`/`Some`/`None`). If 3 or more such match expressions appear as siblings (not nested), flag as interleaved error handling. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect match on Result/Option patterns
(match_expression
  body: (match_block
    (match_arm
      pattern: (tuple_struct_pattern
        type: (identifier) @variant
        (#any-of? @variant "Ok" "Err" "Some" "None"))))) @result_match

;; Detect multiple sibling match-on-Result blocks in a function
(function_item
  body: (block
    (let_declaration
      value: (match_expression) @match1)
    (let_declaration
      value: (match_expression) @match2)
    (let_declaration
      value: (match_expression) @match3)))
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
