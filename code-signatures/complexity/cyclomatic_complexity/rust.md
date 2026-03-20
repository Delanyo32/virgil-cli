# Cyclomatic Complexity -- Rust

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `else if`, `match` arms, loops (`for`, `while`, `loop`), logical operators (`&&`, `||`), and the `?` operator. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Every decision point requires a distinct test case, so high-CC functions impose an outsized testing burden. Rust's rich type system and pattern matching encourage complex branching logic that, while expressive, can produce functions with many independent paths. Elevated cyclomatic complexity correlates with higher defect density and makes refactoring significantly riskier.

## Applicability
- **Relevance**: high
- **Languages covered**: `.rs`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/else branches, match arms, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```rust
fn process_event(event: &Event, config: &Config) -> Result<Action, Error> {
    if event.is_expired() {
        return Err(Error::Expired);
    }

    let action = match event.kind {
        EventKind::Create => {
            if event.priority > 5 || config.force_review {
                Action::Review
            } else if event.size > config.max_auto_size && event.category != Category::Internal {
                Action::QueueLarge
            } else {
                Action::AutoProcess
            }
        }
        EventKind::Update => {
            if event.has_breaking_changes() {
                if config.allow_breaking {
                    Action::ReviewBreaking
                } else {
                    Action::Reject
                }
            } else if event.diff_lines() > 1000 || event.files_changed() > 20 {
                Action::ReviewLarge
            } else {
                Action::AutoProcess
            }
        }
        EventKind::Delete => {
            if event.is_protected() && !config.admin_mode {
                Action::Reject
            } else {
                Action::ConfirmDelete
            }
        }
        EventKind::Archive => Action::Archive,
        _ => return Err(Error::UnknownKind),
    };

    for rule in &config.post_rules {
        if rule.matches(event) && !rule.is_suppressed() {
            return Ok(Action::PostRuleOverride(rule.action()));
        }
    }

    Ok(action)
}
```

### Good Code (Fix)
```rust
fn process_create(event: &Event, config: &Config) -> Action {
    if event.priority > 5 || config.force_review {
        return Action::Review;
    }
    if event.size > config.max_auto_size && event.category != Category::Internal {
        return Action::QueueLarge;
    }
    Action::AutoProcess
}

fn process_update(event: &Event, config: &Config) -> Action {
    if event.has_breaking_changes() {
        return if config.allow_breaking { Action::ReviewBreaking } else { Action::Reject };
    }
    if event.diff_lines() > 1000 || event.files_changed() > 20 {
        return Action::ReviewLarge;
    }
    Action::AutoProcess
}

fn process_delete(event: &Event, config: &Config) -> Action {
    if event.is_protected() && !config.admin_mode {
        Action::Reject
    } else {
        Action::ConfirmDelete
    }
}

fn apply_post_rules(event: &Event, config: &Config, default: Action) -> Action {
    config.post_rules.iter()
        .find(|r| r.matches(event) && !r.is_suppressed())
        .map(|r| Action::PostRuleOverride(r.action()))
        .unwrap_or(default)
}

fn process_event(event: &Event, config: &Config) -> Result<Action, Error> {
    if event.is_expired() {
        return Err(Error::Expired);
    }

    let action = match event.kind {
        EventKind::Create => process_create(event, config),
        EventKind::Update => process_update(event, config),
        EventKind::Delete => process_delete(event, config),
        EventKind::Archive => Action::Archive,
        _ => return Err(Error::UnknownKind),
    };

    Ok(apply_post_rules(event, config, action))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_expression`, `else_clause`, `match_arm`, `for_expression`, `while_expression`, `loop_expression`, `binary_expression` (with `&&`, `||`), `try_expression` (`?` operator)
- **Detection approach**: Count decision points within a function body. Each `if`, `else if`, `match` arm (beyond the first), `for`, `while`, `loop`, `&&`, `||`, and `?` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_item body: (block) @fn_body) @fn

;; Count decision points within function bodies
(if_expression) @decision
(match_arm) @decision
(for_expression) @decision
(while_expression) @decision
(loop_expression) @decision
(binary_expression operator: ["&&" "||"]) @decision
(try_expression) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or match expressions that compound complexity. While Rust's `match` is powerful, nesting matches within if blocks or vice versa quickly erodes readability.

### Bad Code (Anti-pattern)
```rust
fn validate_input(input: &Input) -> Result<Validated, ValidationError> {
    if let Some(header) = &input.header {
        if header.version >= MIN_VERSION {
            match header.format {
                Format::Json => {
                    if let Some(body) = &input.body {
                        if body.len() <= MAX_BODY_SIZE {
                            if body.is_valid_utf8() {
                                Ok(Validated::new(header, body))
                            } else {
                                Err(ValidationError::InvalidEncoding)
                            }
                        } else {
                            Err(ValidationError::BodyTooLarge)
                        }
                    } else {
                        Err(ValidationError::MissingBody)
                    }
                }
                Format::Binary => {
                    if input.checksum_valid() {
                        Ok(Validated::binary(header, &input.raw))
                    } else {
                        Err(ValidationError::ChecksumMismatch)
                    }
                }
                _ => Err(ValidationError::UnsupportedFormat),
            }
        } else {
            Err(ValidationError::VersionTooOld)
        }
    } else {
        Err(ValidationError::MissingHeader)
    }
}
```

### Good Code (Fix)
```rust
fn validate_input(input: &Input) -> Result<Validated, ValidationError> {
    let header = input.header.as_ref().ok_or(ValidationError::MissingHeader)?;
    if header.version < MIN_VERSION {
        return Err(ValidationError::VersionTooOld);
    }

    match header.format {
        Format::Json => validate_json_body(header, input),
        Format::Binary => validate_binary_body(header, input),
        _ => Err(ValidationError::UnsupportedFormat),
    }
}

fn validate_json_body(header: &Header, input: &Input) -> Result<Validated, ValidationError> {
    let body = input.body.as_ref().ok_or(ValidationError::MissingBody)?;
    if body.len() > MAX_BODY_SIZE {
        return Err(ValidationError::BodyTooLarge);
    }
    if !body.is_valid_utf8() {
        return Err(ValidationError::InvalidEncoding);
    }
    Ok(Validated::new(header, body))
}

fn validate_binary_body(header: &Header, input: &Input) -> Result<Validated, ValidationError> {
    if !input.checksum_valid() {
        return Err(ValidationError::ChecksumMismatch);
    }
    Ok(Validated::binary(header, &input.raw))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_expression` containing nested `if_expression` or `match_expression`, `match_arm` containing nested `if_expression`
- **Detection approach**: Track nesting depth of conditional and match expressions within a function body. Walk the AST from each `if_expression` or `match_expression` and count ancestor conditionals within the same function. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested conditionals (3+ levels)
(if_expression
  consequence: (block
    (if_expression
      consequence: (block
        (if_expression) @deeply_nested))))

(if_expression
  consequence: (block
    (expression_statement
      (match_expression
        (match_arm
          (block
            (if_expression) @deeply_nested))))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
