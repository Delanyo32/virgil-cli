# Dead Code -- Python

## Overview
Dead code is code that exists in the codebase but is never executed or referenced — unused functions, unreachable statements after returns, commented-out code blocks, and unused imports or variables.

## Why It's a Code Style Concern
Dead code increases cognitive load during code review, inflates binary size, creates false positive search results, and can mislead developers into thinking features are still active. Python has no compiler warnings for unused code, making dead code especially insidious — it silently accumulates and is only caught by linters or manual review.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: N/A (language-level detection)

---

## Pattern 1: Unused Function/Method

### Description
A function or method defined but never called from anywhere in the codebase.

### Bad Code (Anti-pattern)
```python
def _parse_legacy_config(path):
    """Parse the old YAML config format from v1."""
    with open(path) as f:
        data = yaml.safe_load(f)
    return {k.upper(): v for k, v in data.items()}


def parse_config(path):
    """Parse the current TOML config format."""
    with open(path, "rb") as f:
        return tomllib.load(f)
```

### Good Code (Fix)
```python
def parse_config(path):
    """Parse the current TOML config format."""
    with open(path, "rb") as f:
        return tomllib.load(f)
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`
- **Detection approach**: Collect all function/method definitions and their names. Cross-reference with all `call` nodes and attribute accesses across the file/project. Functions with zero references are candidates. Exclude methods with names starting with `__` (dunder methods), functions decorated with `@property`, `@abstractmethod`, `@app.route`, etc., and functions whose names start with `test_` (test functions). Pay special attention to Python's convention: `_`-prefixed functions are private but may still be called internally.
- **S-expression query sketch**:
  ```scheme
  (function_definition name: (identifier) @fn_name)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unused_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Unreachable Code After Return/Break/Continue

### Description
Code statements that appear after an unconditional return, break, continue, or raise — they can never execute.

### Bad Code (Anti-pattern)
```python
def get_user_role(user_id):
    if user_id == 0:
        return "admin"
        logger.info("Admin user accessed")  # unreachable

    raise ValueError(f"Unknown user: {user_id}")
    return "guest"  # unreachable
```

### Good Code (Fix)
```python
def get_user_role(user_id):
    if user_id == 0:
        return "admin"

    raise ValueError(f"Unknown user: {user_id}")
```

### Tree-sitter Detection Strategy
- **Target node types**: `return_statement`, `break_statement`, `continue_statement`, `raise_statement`
- **Detection approach**: For each early-exit statement, check if there are sibling statements after it in the same `block`. Those siblings are unreachable. In Python, indentation defines blocks, so tree-sitter's `block` node accurately represents scope. Exclude statements inside `try`/`except`/`finally` where flow may differ.
- **S-expression query sketch**:
  ```scheme
  (block
    (return_statement) @exit
    .
    (_) @unreachable)
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `unreachable_after_return`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Commented-Out Code

### Description
Large blocks of commented-out code left in the source, typically from debugging or removed features.

### Bad Code (Anti-pattern)
```python
def process_payment(order):
    # def validate_card(card_number):
    #     if len(card_number) != 16:
    #         raise ValueError("Invalid card")
    #     checksum = sum(int(d) for d in card_number)
    #     return checksum % 10 == 0
    #
    # if not validate_card(order.card):
    #     return PaymentResult(success=False, error="Invalid card")

    return gateway.charge(order.total, order.card)
```

### Good Code (Fix)
```python
def process_payment(order):
    return gateway.charge(order.total, order.card)
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment`
- **Detection approach**: Find comment nodes whose content matches Python code patterns (contains `def `, `class `, `import `, `if `, `for `, `return `, assignment with `=`, function calls with parentheses). Flag blocks of 5+ consecutive comment lines that look like code. Distinguish from docstrings and descriptive comments by checking for syntactic structures.
- **S-expression query sketch**:
  ```scheme
  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `dead_code`
- **Pattern name**: `commented_out_code`
- **Severity**: info
- **Confidence**: low
