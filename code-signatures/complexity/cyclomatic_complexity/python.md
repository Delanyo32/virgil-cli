# Cyclomatic Complexity -- Python

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `elif`, `for`, `while`, boolean operators (`and`, `or`), `except` clauses, and conditional expressions within comprehensions. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Each decision point demands a dedicated test case to cover the corresponding branch, so high-CC functions require a disproportionate amount of testing effort. Python's reliance on indentation makes deeply branched code particularly hard to read and mentally trace. Studies consistently link elevated cyclomatic complexity to higher defect rates and longer debugging cycles.

## Applicability
- **Relevance**: high
- **Languages covered**: `.py`, `.pyi`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/elif branches, loops with conditional breaks, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```python
def process_transaction(txn):
    result = None
    if txn.type == "purchase":
        if txn.amount > 10000 or txn.flagged:
            result = "manual_review"
        elif txn.amount > 5000 and txn.currency != "USD":
            result = "fx_review"
        elif txn.method == "wire":
            if txn.destination_country in SANCTIONED_COUNTRIES:
                result = "blocked"
            else:
                result = "wire_processing"
        elif txn.method == "card":
            result = "auto_approve"
        else:
            result = "unknown_method"
    elif txn.type == "refund":
        if txn.original_txn and txn.original_txn.settled:
            if txn.amount <= txn.original_txn.amount:
                result = "refund_approved"
            else:
                result = "refund_exceeds_original"
        else:
            result = "refund_denied"
    elif txn.type == "chargeback":
        result = "dispute_review"
    else:
        result = "unknown_type"

    for rule in txn.compliance_rules:
        if rule.applies(txn) and not rule.is_waived:
            result = "compliance_hold"
            break

    return result
```

### Good Code (Fix)
```python
def _resolve_purchase(txn):
    if txn.amount > 10000 or txn.flagged:
        return "manual_review"
    if txn.amount > 5000 and txn.currency != "USD":
        return "fx_review"
    return _resolve_purchase_method(txn)


def _resolve_purchase_method(txn):
    handlers = {
        "wire": lambda t: "blocked" if t.destination_country in SANCTIONED_COUNTRIES else "wire_processing",
        "card": lambda _: "auto_approve",
    }
    return handlers.get(txn.method, lambda _: "unknown_method")(txn)


def _resolve_refund(txn):
    if not txn.original_txn or not txn.original_txn.settled:
        return "refund_denied"
    if txn.amount > txn.original_txn.amount:
        return "refund_exceeds_original"
    return "refund_approved"


_TYPE_HANDLERS = {
    "purchase": _resolve_purchase,
    "refund": _resolve_refund,
    "chargeback": lambda _: "dispute_review",
}


def process_transaction(txn):
    handler = _TYPE_HANDLERS.get(txn.type)
    result = handler(txn) if handler else "unknown_type"
    return _apply_compliance(txn, result)


def _apply_compliance(txn, result):
    if any(r.applies(txn) and not r.is_waived for r in txn.compliance_rules):
        return "compliance_hold"
    return result
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `elif_clause`, `for_statement`, `while_statement`, `boolean_operator` (`and`, `or`), `except_clause`, `conditional_expression`, `if_clause` (comprehension filter)
- **Detection approach**: Count decision points within a function body. Each `if`, `elif`, `for`, `while`, `and`, `or`, `except`, ternary (`x if cond else y`), and comprehension filter (`if` inside list/dict/set comprehension) adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_definition body: (block) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(elif_clause) @decision
(for_statement) @decision
(while_statement) @decision
(boolean_operator) @decision
(except_clause) @decision
(conditional_expression) @decision
(list_comprehension (if_clause) @decision)
(set_comprehension (if_clause) @decision)
(dictionary_comprehension (if_clause) @decision)
(generator_expression (if_clause) @decision)
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/elif/else blocks that compound complexity. Python's indentation-based syntax makes deep nesting especially painful to read and maintain.

### Bad Code (Anti-pattern)
```python
def authorize_request(request, user, config):
    if request is not None:
        if user is not None:
            if user.is_authenticated:
                if config.require_mfa:
                    if user.mfa_verified:
                        if user.has_permission(request.resource):
                            return {"allowed": True}
                        else:
                            return {"allowed": False, "reason": "no_permission"}
                    else:
                        return {"allowed": False, "reason": "mfa_required"}
                else:
                    if user.has_permission(request.resource):
                        return {"allowed": True}
                    else:
                        return {"allowed": False, "reason": "no_permission"}
            else:
                return {"allowed": False, "reason": "not_authenticated"}
        else:
            return {"allowed": False, "reason": "no_user"}
    else:
        return {"allowed": False, "reason": "no_request"}
```

### Good Code (Fix)
```python
def authorize_request(request, user, config):
    if request is None:
        return {"allowed": False, "reason": "no_request"}
    if user is None:
        return {"allowed": False, "reason": "no_user"}
    if not user.is_authenticated:
        return {"allowed": False, "reason": "not_authenticated"}
    if config.require_mfa and not user.mfa_verified:
        return {"allowed": False, "reason": "mfa_required"}
    if not user.has_permission(request.resource):
        return {"allowed": False, "reason": "no_permission"}

    return {"allowed": True}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `block` body
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function boundary. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested if statements (3+ levels)
(if_statement
  consequence: (block
    (if_statement
      consequence: (block
        (if_statement) @deeply_nested))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
