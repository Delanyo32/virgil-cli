# Comment Ratio -- Python

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .py, .pyi
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```python
def reconcile_transactions(ledger, bank_statements, tolerance=0.01):
    matched = []
    unmatched_ledger = []
    for entry in ledger:
        found = False
        for stmt in bank_statements:
            if stmt.matched:
                continue
            if abs(entry.amount - stmt.amount) <= tolerance:
                if entry.date == stmt.date or (entry.date - stmt.date).days <= 3:
                    matched.append((entry, stmt))
                    stmt.matched = True
                    found = True
                    break
                elif entry.reference and entry.reference in stmt.description:
                    matched.append((entry, stmt))
                    stmt.matched = True
                    found = True
                    break
        if not found:
            if entry.amount < 0 and any(
                abs(entry.amount + s.amount) <= tolerance
                for s in bank_statements if not s.matched
            ):
                reversal = next(
                    s for s in bank_statements
                    if not s.matched and abs(entry.amount + s.amount) <= tolerance
                )
                matched.append((entry, reversal))
                reversal.matched = True
            else:
                unmatched_ledger.append(entry)
    return matched, unmatched_ledger
```

### Good Code (Fix)
```python
def reconcile_transactions(ledger, bank_statements, tolerance=0.01):
    """Reconcile ledger entries against bank statements using multi-pass matching."""
    matched = []
    unmatched_ledger = []

    for entry in ledger:
        found = False
        for stmt in bank_statements:
            if stmt.matched:
                continue

            if abs(entry.amount - stmt.amount) <= tolerance:
                # Primary match: same amount within tolerance and date within
                # the 3-day clearing window banks typically use
                if entry.date == stmt.date or (entry.date - stmt.date).days <= 3:
                    matched.append((entry, stmt))
                    stmt.matched = True
                    found = True
                    break
                # Fallback: reference number embedded in bank description
                # handles delayed or batched postings
                elif entry.reference and entry.reference in stmt.description:
                    matched.append((entry, stmt))
                    stmt.matched = True
                    found = True
                    break

        if not found:
            # Detect reversal pairs: a negative entry that exactly cancels
            # a positive bank statement (common with refunds and chargebacks)
            if entry.amount < 0 and any(
                abs(entry.amount + s.amount) <= tolerance
                for s in bank_statements if not s.matched
            ):
                reversal = next(
                    s for s in bank_statements
                    if not s.matched and abs(entry.amount + s.amount) <= tolerance
                )
                matched.append((entry, reversal))
                reversal.matched = True
            else:
                unmatched_ledger.append(entry)

    return matched, unmatched_ledger
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` for function bodies; `comment` for `#` comments; `expression_statement > string` for docstrings (`"""..."""`)
- **Detection approach**: Count comment lines (including docstring lines) and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments/docstrings within it
  (function_definition
    body: (block) @function.body)

  (comment) @comment

  ;; Docstrings as first statement in function body
  (function_definition
    body: (block
      .
      (expression_statement
        (string) @docstring)))
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
```python
def update_user_profile(user, data):
    # Get the username
    username = data.get("username")

    # Check if username is not None
    if username is not None:
        # Set user's name to username
        user.name = username

    # Get the email
    email = data.get("email")

    # Check if email is not None
    if email is not None:
        # Set user's email to email
        user.email = email

    # Save the user
    user.save()

    # Return the user
    return user
```

### Good Code (Fix)
```python
def update_user_profile(user, data):
    """Partially update user profile fields, skipping any not provided."""
    username = data.get("username")
    if username is not None:
        user.name = username

    email = data.get("email")
    if email is not None:
        user.email = email

    # save() triggers the post-save webhook for CRM sync
    user.save()
    return user
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `expression_statement`, `assignment`, `return_statement`, `if_statement`
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
