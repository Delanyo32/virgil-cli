# Duplicate Code -- Python

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters.

### Bad Code (Anti-pattern)
```python
def process_user_data(records):
    cleaned = []
    for record in records:
        if record.get("name") is None or record.get("email") is None:
            logger.warning(f"Skipping invalid user record: {record['id']}")
            continue
        normalized = {
            "id": record["id"],
            "name": record["name"].strip().lower(),
            "email": record["email"].strip().lower(),
            "created_at": datetime.utcnow(),
        }
        cleaned.append(normalized)
    df = pd.DataFrame(cleaned)
    df.drop_duplicates(subset=["email"], inplace=True)
    return df


def process_vendor_data(records):
    cleaned = []
    for record in records:
        if record.get("name") is None or record.get("email") is None:
            logger.warning(f"Skipping invalid vendor record: {record['id']}")
            continue
        normalized = {
            "id": record["id"],
            "name": record["name"].strip().lower(),
            "email": record["email"].strip().lower(),
            "created_at": datetime.utcnow(),
        }
        cleaned.append(normalized)
    df = pd.DataFrame(cleaned)
    df.drop_duplicates(subset=["email"], inplace=True)
    return df
```

### Good Code (Fix)
```python
def process_entity_data(records, entity_type="entity"):
    cleaned = []
    for record in records:
        if record.get("name") is None or record.get("email") is None:
            logger.warning(f"Skipping invalid {entity_type} record: {record['id']}")
            continue
        normalized = {
            "id": record["id"],
            "name": record["name"].strip().lower(),
            "email": record["email"].strip().lower(),
            "created_at": datetime.utcnow(),
        }
        cleaned.append(normalized)
    df = pd.DataFrame(cleaned)
    df.drop_duplicates(subset=["email"], inplace=True)
    return df
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `block`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_definition
  name: (identifier) @func_name
  body: (block) @func_body)
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a function or across methods in the same class, often due to copy-paste during development. Common in data processing pipelines and repeated pandas transformations.

### Bad Code (Anti-pattern)
```python
class ReportGenerator:
    def generate_sales_report(self, start_date, end_date):
        df = pd.read_sql(f"SELECT * FROM sales WHERE date BETWEEN '{start_date}' AND '{end_date}'", self.engine)
        df["amount"] = df["amount"].astype(float)
        df["date"] = pd.to_datetime(df["date"])
        df = df.dropna(subset=["amount", "date"])
        df["month"] = df["date"].dt.to_period("M")
        summary = df.groupby("month")["amount"].agg(["sum", "mean", "count"])
        summary.to_csv(f"sales_report_{start_date}_{end_date}.csv")
        return summary

    def generate_returns_report(self, start_date, end_date):
        df = pd.read_sql(f"SELECT * FROM returns WHERE date BETWEEN '{start_date}' AND '{end_date}'", self.engine)
        df["amount"] = df["amount"].astype(float)
        df["date"] = pd.to_datetime(df["date"])
        df = df.dropna(subset=["amount", "date"])
        df["month"] = df["date"].dt.to_period("M")
        summary = df.groupby("month")["amount"].agg(["sum", "mean", "count"])
        summary.to_csv(f"returns_report_{start_date}_{end_date}.csv")
        return summary
```

### Good Code (Fix)
```python
class ReportGenerator:
    def _build_period_summary(self, table, start_date, end_date):
        df = pd.read_sql(f"SELECT * FROM {table} WHERE date BETWEEN '{start_date}' AND '{end_date}'", self.engine)
        df["amount"] = df["amount"].astype(float)
        df["date"] = pd.to_datetime(df["date"])
        df = df.dropna(subset=["amount", "date"])
        df["month"] = df["date"].dt.to_period("M")
        return df.groupby("month")["amount"].agg(["sum", "mean", "count"])

    def generate_sales_report(self, start_date, end_date):
        summary = self._build_period_summary("sales", start_date, end_date)
        summary.to_csv(f"sales_report_{start_date}_{end_date}.csv")
        return summary

    def generate_returns_report(self, start_date, end_date):
        summary = self._build_period_summary("returns", start_date, end_date)
        summary.to_csv(f"returns_report_{start_date}_{end_date}.csv")
        return summary
```

### Tree-sitter Detection Strategy
- **Target node types**: `block`, `expression_statement`, `assignment`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(block
  (_) @stmt)

(function_definition
  body: (block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
