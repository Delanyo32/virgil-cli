# Cognitive Complexity -- Python

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, except, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py`, `.pyi`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/except, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```python
def process_records(records, config):
    results = []
    for record in records:
        if record.is_valid:
            try:
                if record.category == "priority":
                    for field in record.fields:
                        if field.name in config.required_fields:
                            if field.value is None or field.value == "":
                                continue
                            results.append(transform_field(field))
                        else:
                            break
                else:
                    if record.has_fallback:
                        results.append(default_transform(record))
            except ValidationError as e:
                if config.strict:
                    raise
                results.append(error_result(record, e))
    return results
```

### Good Code (Fix)
```python
def _is_usable_field(field, required_fields):
    if field.name not in required_fields:
        return None  # signal to stop
    if field.value is None or field.value == "":
        return False  # signal to skip
    return True

def _process_priority_record(record, config):
    results = []
    for field in record.fields:
        usable = _is_usable_field(field, config.required_fields)
        if usable is None:
            break
        if not usable:
            continue
        results.append(transform_field(field))
    return results

def _process_single_record(record, config):
    if not record.is_valid:
        return []
    if record.category == "priority":
        return _process_priority_record(record, config)
    if record.has_fallback:
        return [default_transform(record)]
    return []

def process_records(records, config):
    results = []
    for record in records:
        try:
            results.extend(_process_single_record(record, config))
        except ValidationError as e:
            if config.strict:
                raise
            results.append(error_result(record, e))
    return results
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `while_statement`, `try_statement`, `except_clause`, `with_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and `raise` statements add 1 each for flow disruption. `elif` and `else` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_definition body: (block) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(while_statement) @nesting
(try_statement) @nesting
(with_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(raise_statement) @flow_break

;; Else/elif/except increments (breaks linear flow)
(elif_clause) @flow_break
(else_clause) @flow_break
(except_clause) @flow_break
(finally_clause) @flow_break
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/except per operation that fragments the readable logic flow.

### Bad Code (Anti-pattern)
```python
def sync_user_data(user_id):
    try:
        user = fetch_user(user_id)
    except ConnectionError:
        return {"success": False, "error": "fetch_failed"}

    try:
        profile = fetch_profile(user.profile_id)
    except ConnectionError:
        return {"success": False, "error": "profile_failed"}

    try:
        preferences = load_preferences(user.id)
    except FileNotFoundError:
        preferences = default_preferences()

    try:
        merged = merge_data(user, profile, preferences)
    except ValueError:
        return {"success": False, "error": "merge_failed"}

    try:
        save_to_cache(merged)
    except CacheError:
        logging.warning("Cache save failed, continuing")

    try:
        notify_services(merged)
    except NotificationError:
        return {"success": False, "error": "notify_failed"}

    return {"success": True, "data": merged}
```

### Good Code (Fix)
```python
def _load_preferences_safe(user_id):
    try:
        return load_preferences(user_id)
    except FileNotFoundError:
        return default_preferences()

def _save_to_cache_safe(data):
    try:
        save_to_cache(data)
    except CacheError:
        logging.warning("Cache save failed, continuing")

def sync_user_data(user_id):
    try:
        user = fetch_user(user_id)
        profile = fetch_profile(user.profile_id)
        preferences = _load_preferences_safe(user.id)
        merged = merge_data(user, profile, preferences)
        _save_to_cache_safe(merged)
        notify_services(merged)
        return {"success": True, "data": merged}
    except (ConnectionError, ValueError, NotificationError) as e:
        stage = identify_failure_stage(e)
        return {"success": False, "error": stage}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `except_clause`
- **Detection approach**: Count `try_statement` nodes within a single function body. If 3 or more try/except blocks appear as siblings (not nested), flag as interleaved error handling. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a function
(function_definition
  body: (block
    (try_statement) @try1
    (try_statement) @try2
    (try_statement) @try3))

;; Detect except clauses
(except_clause) @error_handler
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
