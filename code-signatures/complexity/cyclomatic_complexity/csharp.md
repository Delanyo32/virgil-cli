# Cyclomatic Complexity -- C#

## Overview
Cyclomatic complexity measures the number of independent execution paths through a method by counting decision points such as `if`, `else if`, `switch` cases, loops (`for`, `foreach`, `while`, `do-while`), logical operators (`&&`, `||`), ternary expressions (`?:`), null-coalescing operators (`??`), and `catch` clauses. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Each decision point demands its own test case for branch coverage, making high-CC methods expensive to test thoroughly. C#'s rich feature set -- pattern matching, null-coalescing, LINQ -- can obscure branching when overused in a single method. Empirical data consistently links elevated cyclomatic complexity to increased defect density and maintainability challenges.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Methods with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```csharp
public string ClassifyCustomer(Customer customer, AppConfig config)
{
    if (customer == null)
        return "invalid";

    if (customer.Type == CustomerType.Enterprise)
    {
        if (customer.AnnualRevenue > 1000000 || config.ForceReview)
        {
            if (customer.Region == "APAC" && customer.YearsActive < 2)
                return "new_enterprise_apac";
            else if (customer.Region == "EMEA" || customer.Region == "LATAM")
                return "international_enterprise";
            else
                return "domestic_enterprise";
        }
        else if (customer.AnnualRevenue > 500000 && customer.IsVerified)
        {
            return "mid_enterprise";
        }
        else
        {
            return "small_enterprise";
        }
    }
    else if (customer.Type == CustomerType.SMB)
    {
        if (customer.EmployeeCount > 50 ?? false)
            return customer.IsVerified ? "verified_smb" : "unverified_smb";
        else
            return "micro_smb";
    }
    else if (customer.Type == CustomerType.Individual)
    {
        switch (customer.Tier)
        {
            case "gold":
                return "premium_individual";
            case "silver":
                return customer.YearsActive > 5 ? "loyal_individual" : "standard_individual";
            case "bronze":
                return "basic_individual";
            default:
                return "untiered_individual";
        }
    }
    else
    {
        return "unknown_type";
    }
}
```

### Good Code (Fix)
```csharp
public string ClassifyCustomer(Customer customer, AppConfig config)
{
    if (customer == null)
        return "invalid";

    return customer.Type switch
    {
        CustomerType.Enterprise => ClassifyEnterprise(customer, config),
        CustomerType.SMB => ClassifySmb(customer),
        CustomerType.Individual => ClassifyIndividual(customer),
        _ => "unknown_type",
    };
}

private string ClassifyEnterprise(Customer customer, AppConfig config)
{
    if (customer.AnnualRevenue > 1000000 || config.ForceReview)
        return ClassifyLargeEnterprise(customer);
    if (customer.AnnualRevenue > 500000 && customer.IsVerified)
        return "mid_enterprise";
    return "small_enterprise";
}

private string ClassifyLargeEnterprise(Customer customer)
{
    if (customer.Region == "APAC" && customer.YearsActive < 2)
        return "new_enterprise_apac";
    if (customer.Region is "EMEA" or "LATAM")
        return "international_enterprise";
    return "domestic_enterprise";
}

private string ClassifySmb(Customer customer)
{
    if ((customer.EmployeeCount ?? 0) <= 50)
        return "micro_smb";
    return customer.IsVerified ? "verified_smb" : "unverified_smb";
}

private string ClassifyIndividual(Customer customer) => customer.Tier switch
{
    "gold" => "premium_individual",
    "silver" => customer.YearsActive > 5 ? "loyal_individual" : "standard_individual",
    "bronze" => "basic_individual",
    _ => "untiered_individual",
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_clause`, `switch_section` (switch case), `for_statement`, `for_each_statement`, `while_statement`, `do_statement`, `binary_expression` (with `&&`, `||`, `??`), `conditional_expression` (`?:`), `catch_clause`
- **Detection approach**: Count decision points within a method body. Each `if`, `else if`, `case`, `for`, `foreach`, `while`, `do-while`, `&&`, `||`, `??`, `?:`, and `catch` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find method bodies
(method_declaration body: (block) @method_body) @method
(constructor_declaration body: (block) @method_body) @method
(local_function_statement body: (block) @method_body) @method

;; Count decision points within method bodies
(if_statement) @decision
(switch_section) @decision
(for_statement) @decision
(for_each_statement) @decision
(while_statement) @decision
(do_statement) @decision
(conditional_expression) @decision
(catch_clause) @decision
(binary_expression operator: ["&&" "||" "??"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or switch statements that compound complexity. C#'s pattern matching and switch expressions can help flatten these, but legacy code often exhibits deep nesting.

### Bad Code (Anti-pattern)
```csharp
public ActionResult ProcessOrder(OrderRequest request)
{
    if (request != null)
    {
        if (ModelState.IsValid)
        {
            var user = _userService.GetUser(request.UserId);
            if (user != null)
            {
                if (user.IsActive)
                {
                    if (_inventoryService.IsAvailable(request.ProductId))
                    {
                        try
                        {
                            var order = _orderService.Create(request);
                            if (order != null)
                            {
                                return Ok(order);
                            }
                            else
                            {
                                return StatusCode(500, "order creation failed");
                            }
                        }
                        catch (Exception ex)
                        {
                            return StatusCode(500, ex.Message);
                        }
                    }
                    else
                    {
                        return BadRequest("product unavailable");
                    }
                }
                else
                {
                    return Forbid("user inactive");
                }
            }
            else
            {
                return NotFound("user not found");
            }
        }
        else
        {
            return BadRequest(ModelState);
        }
    }
    else
    {
        return BadRequest("null request");
    }
}
```

### Good Code (Fix)
```csharp
public ActionResult ProcessOrder(OrderRequest request)
{
    if (request == null)
        return BadRequest("null request");
    if (!ModelState.IsValid)
        return BadRequest(ModelState);

    var user = _userService.GetUser(request.UserId);
    if (user == null)
        return NotFound("user not found");
    if (!user.IsActive)
        return Forbid("user inactive");
    if (!_inventoryService.IsAvailable(request.ProductId))
        return BadRequest("product unavailable");

    return CreateOrder(request);
}

private ActionResult CreateOrder(OrderRequest request)
{
    try
    {
        var order = _orderService.Create(request);
        return order != null ? Ok(order) : StatusCode(500, "order creation failed");
    }
    catch (Exception ex)
    {
        return StatusCode(500, ex.Message);
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
