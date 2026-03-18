# Cyclomatic Complexity -- Java

## Overview
Cyclomatic complexity measures the number of independent execution paths through a method by counting decision points such as `if`, `else if`, `switch` cases, loops (`for`, `while`, `do-while`), logical operators (`&&`, `||`), ternary expressions (`?:`), and `catch` clauses. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Each decision point requires a dedicated test case to achieve branch coverage, so high-CC methods require a disproportionate amount of unit testing. Java's verbose syntax and pervasive use of checked exceptions can amplify CC through repeated try/catch blocks. Studies consistently associate elevated cyclomatic complexity with higher defect density and longer mean time to resolution.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Methods with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```java
public String evaluateLoan(Application app) {
    if (app == null) {
        return "invalid";
    }
    if (app.getType().equals("mortgage")) {
        if (app.getCreditScore() >= 750 && app.getDebtRatio() < 0.3) {
            if (app.getDownPayment() >= app.getLoanAmount() * 0.2) {
                return "auto_approve";
            } else if (app.getDownPayment() >= app.getLoanAmount() * 0.1 || app.hasInsurance()) {
                return "conditional_approve";
            } else {
                return "manual_review";
            }
        } else if (app.getCreditScore() >= 650) {
            return "enhanced_review";
        } else {
            return "decline";
        }
    } else if (app.getType().equals("auto")) {
        if (app.getLoanAmount() > 50000 && !app.isExistingCustomer()) {
            return "manual_review";
        } else {
            return app.getCreditScore() >= 700 ? "auto_approve" : "standard_review";
        }
    } else if (app.getType().equals("personal")) {
        switch (app.getRiskCategory()) {
            case "low":
                return "auto_approve";
            case "medium":
                return app.getEmploymentYears() > 2 ? "auto_approve" : "review";
            case "high":
                return "decline";
            default:
                return "manual_review";
        }
    } else {
        return "unsupported_type";
    }
}
```

### Good Code (Fix)
```java
public String evaluateLoan(Application app) {
    if (app == null) {
        return "invalid";
    }

    return switch (app.getType()) {
        case "mortgage" -> evaluateMortgage(app);
        case "auto" -> evaluateAuto(app);
        case "personal" -> evaluatePersonal(app);
        default -> "unsupported_type";
    };
}

private String evaluateMortgage(Application app) {
    if (app.getCreditScore() < 650) {
        return "decline";
    }
    if (app.getCreditScore() < 750 || app.getDebtRatio() >= 0.3) {
        return "enhanced_review";
    }
    return evaluateMortgageDownPayment(app);
}

private String evaluateMortgageDownPayment(Application app) {
    double ratio = app.getDownPayment() / app.getLoanAmount();
    if (ratio >= 0.2) return "auto_approve";
    if (ratio >= 0.1 || app.hasInsurance()) return "conditional_approve";
    return "manual_review";
}

private String evaluateAuto(Application app) {
    if (app.getLoanAmount() > 50000 && !app.isExistingCustomer()) {
        return "manual_review";
    }
    return app.getCreditScore() >= 700 ? "auto_approve" : "standard_review";
}

private String evaluatePersonal(Application app) {
    return switch (app.getRiskCategory()) {
        case "low" -> "auto_approve";
        case "medium" -> app.getEmploymentYears() > 2 ? "auto_approve" : "review";
        case "high" -> "decline";
        default -> "manual_review";
    };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `switch_block_statement_group` (switch case), `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement`, `binary_expression` (with `&&`, `||`), `ternary_expression`, `catch_clause`
- **Detection approach**: Count decision points within a method body. Each `if`, `else if`, `case`, `for`, `for-each`, `while`, `do-while`, `&&`, `||`, `?:`, and `catch` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find method bodies
(method_declaration body: (block) @method_body) @method
(constructor_declaration body: (constructor_body) @method_body) @method

;; Count decision points within method bodies
(if_statement) @decision
(switch_block_statement_group) @decision
(for_statement) @decision
(enhanced_for_statement) @decision
(while_statement) @decision
(do_statement) @decision
(ternary_expression) @decision
(catch_clause) @decision
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
Deeply nested if/else or switch statements that compound complexity. Java's verbose syntax makes deep nesting particularly hard to follow.

### Bad Code (Anti-pattern)
```java
public Response handleRequest(Request request) {
    if (request != null) {
        if (request.isValid()) {
            User user = userService.findById(request.getUserId());
            if (user != null) {
                if (user.isActive()) {
                    if (user.hasPermission(request.getAction())) {
                        try {
                            Result result = service.execute(request);
                            if (result.isSuccess()) {
                                return Response.ok(result.getData());
                            } else {
                                return Response.error("execution failed");
                            }
                        } catch (ServiceException e) {
                            return Response.error("service error");
                        }
                    } else {
                        return Response.forbidden("no permission");
                    }
                } else {
                    return Response.forbidden("user inactive");
                }
            } else {
                return Response.notFound("user not found");
            }
        } else {
            return Response.badRequest("invalid request");
        }
    } else {
        return Response.badRequest("null request");
    }
}
```

### Good Code (Fix)
```java
public Response handleRequest(Request request) {
    if (request == null || !request.isValid()) {
        return Response.badRequest(request == null ? "null request" : "invalid request");
    }

    User user = userService.findById(request.getUserId());
    if (user == null) {
        return Response.notFound("user not found");
    }
    if (!user.isActive()) {
        return Response.forbidden("user inactive");
    }
    if (!user.hasPermission(request.getAction())) {
        return Response.forbidden("no permission");
    }

    return executeRequest(request);
}

private Response executeRequest(Request request) {
    try {
        Result result = service.execute(request);
        return result.isSuccess()
            ? Response.ok(result.getData())
            : Response.error("execution failed");
    } catch (ServiceException e) {
        return Response.error("service error");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `block` body
- **Detection approach**: Track nesting depth of conditional statements within a method body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same method boundary. Flag when nesting depth exceeds 3 levels.
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
