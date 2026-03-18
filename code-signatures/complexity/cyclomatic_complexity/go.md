# Cyclomatic Complexity -- Go

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `else if`, `switch` cases, `for` loops, and logical operators (`&&`, `||`). High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Every decision point introduces a branch that needs its own test case, so functions with high CC impose a heavy testing burden. Go's idiomatic error handling with repeated `if err != nil` checks can inflate CC quickly even in straightforward code, making it especially important to monitor. Elevated cyclomatic complexity correlates with higher defect rates and makes code harder to review and maintain.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths. In Go, idiomatic error handling can naturally inflate CC.

### Bad Code (Anti-pattern)
```go
func ProcessRecord(r *Record, cfg *Config) (string, error) {
    if r == nil {
        return "", errors.New("nil record")
    }
    if r.Type == "invoice" {
        if r.Amount > 10000 || cfg.ForceReview {
            if r.Currency != "USD" && r.Currency != "EUR" {
                return "fx_review", nil
            } else if r.Customer.IsVIP {
                return "vip_review", nil
            } else {
                return "standard_review", nil
            }
        } else if r.Amount > 5000 && r.Customer.Region == "EMEA" {
            return "regional_review", nil
        } else {
            return "auto_approve", nil
        }
    } else if r.Type == "credit_note" {
        if r.LinkedInvoice == "" {
            return "", errors.New("missing linked invoice")
        }
        if r.Amount > r.LinkedAmount || r.IsDisputed {
            return "dispute_review", nil
        }
        return "auto_credit", nil
    } else if r.Type == "payment" {
        switch r.Method {
        case "wire":
            if r.International && cfg.ComplianceMode {
                return "compliance_check", nil
            }
            return "wire_processing", nil
        case "card":
            return "card_processing", nil
        case "ach":
            return "ach_processing", nil
        default:
            return "", fmt.Errorf("unknown method: %s", r.Method)
        }
    } else {
        return "", fmt.Errorf("unknown type: %s", r.Type)
    }
}
```

### Good Code (Fix)
```go
func ProcessRecord(r *Record, cfg *Config) (string, error) {
    if r == nil {
        return "", errors.New("nil record")
    }

    handlers := map[string]func(*Record, *Config) (string, error){
        "invoice":     processInvoice,
        "credit_note": processCreditNote,
        "payment":     processPayment,
    }

    handler, ok := handlers[r.Type]
    if !ok {
        return "", fmt.Errorf("unknown type: %s", r.Type)
    }
    return handler(r, cfg)
}

func processInvoice(r *Record, cfg *Config) (string, error) {
    if r.Amount > 10000 || cfg.ForceReview {
        return reviewInvoice(r)
    }
    if r.Amount > 5000 && r.Customer.Region == "EMEA" {
        return "regional_review", nil
    }
    return "auto_approve", nil
}

func reviewInvoice(r *Record) (string, error) {
    if r.Currency != "USD" && r.Currency != "EUR" {
        return "fx_review", nil
    }
    if r.Customer.IsVIP {
        return "vip_review", nil
    }
    return "standard_review", nil
}

func processCreditNote(r *Record, _ *Config) (string, error) {
    if r.LinkedInvoice == "" {
        return "", errors.New("missing linked invoice")
    }
    if r.Amount > r.LinkedAmount || r.IsDisputed {
        return "dispute_review", nil
    }
    return "auto_credit", nil
}

func processPayment(r *Record, cfg *Config) (string, error) {
    switch r.Method {
    case "wire":
        return processWire(r, cfg)
    case "card":
        return "card_processing", nil
    case "ach":
        return "ach_processing", nil
    default:
        return "", fmt.Errorf("unknown method: %s", r.Method)
    }
}

func processWire(r *Record, cfg *Config) (string, error) {
    if r.International && cfg.ComplianceMode {
        return "compliance_check", nil
    }
    return "wire_processing", nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `expression_case` (switch case), `default_case`, `for_statement`, `binary_expression` (with `&&`, `||`)
- **Detection approach**: Count decision points within a function body. Each `if`, `else if`, `case`, `default`, `for`, `&&`, and `||` adds 1 to CC. Note that Go has no `while` or `do-while` -- `for` is the only loop construct. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_declaration body: (block) @fn_body) @fn
(method_declaration body: (block) @fn_body) @fn
(func_literal body: (block) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(expression_case) @decision
(default_case) @decision
(for_statement) @decision
(binary_expression operator: ["&&" "||"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or switch statements that compound complexity. In Go, nested `if err != nil` checks and conditional type assertions often create excessive depth.

### Bad Code (Anti-pattern)
```go
func HandleRequest(w http.ResponseWriter, r *http.Request) {
    if r.Method == "POST" {
        body, err := io.ReadAll(r.Body)
        if err == nil {
            var req RequestPayload
            if err := json.Unmarshal(body, &req); err == nil {
                if req.UserID != "" {
                    user, err := db.FindUser(req.UserID)
                    if err == nil {
                        if user.IsActive {
                            result, err := processForUser(user, &req)
                            if err == nil {
                                json.NewEncoder(w).Encode(result)
                            } else {
                                http.Error(w, "processing failed", 500)
                            }
                        } else {
                            http.Error(w, "user inactive", 403)
                        }
                    } else {
                        http.Error(w, "user not found", 404)
                    }
                } else {
                    http.Error(w, "missing user_id", 400)
                }
            } else {
                http.Error(w, "invalid json", 400)
            }
        } else {
            http.Error(w, "read error", 500)
        }
    } else {
        http.Error(w, "method not allowed", 405)
    }
}
```

### Good Code (Fix)
```go
func HandleRequest(w http.ResponseWriter, r *http.Request) {
    if r.Method != "POST" {
        http.Error(w, "method not allowed", 405)
        return
    }

    body, err := io.ReadAll(r.Body)
    if err != nil {
        http.Error(w, "read error", 500)
        return
    }

    var req RequestPayload
    if err := json.Unmarshal(body, &req); err != nil {
        http.Error(w, "invalid json", 400)
        return
    }

    if req.UserID == "" {
        http.Error(w, "missing user_id", 400)
        return
    }

    user, err := db.FindUser(req.UserID)
    if err != nil {
        http.Error(w, "user not found", 404)
        return
    }

    if !user.IsActive {
        http.Error(w, "user inactive", 403)
        return
    }

    result, err := processForUser(user, &req)
    if err != nil {
        http.Error(w, "processing failed", 500)
        return
    }

    json.NewEncoder(w).Encode(result)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `block` consequence
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function. Flag when nesting depth exceeds 3 levels.
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
