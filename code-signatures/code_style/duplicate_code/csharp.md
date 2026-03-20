# Duplicate Code -- C#

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more methods with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared method with parameters. Common in duplicate controller actions.

### Bad Code (Anti-pattern)
```csharp
public class OrderController : ControllerBase
{
    [HttpPost("users/orders")]
    public async Task<IActionResult> CreateUserOrder([FromBody] UserOrderRequest request)
    {
        if (string.IsNullOrWhiteSpace(request.Name) || string.IsNullOrWhiteSpace(request.Email))
            return BadRequest("Name and email are required");

        var normalizedName = request.Name.Trim().ToLowerInvariant();
        var order = new UserOrder
        {
            Name = normalizedName,
            Email = request.Email,
            Amount = request.Quantity * request.Price,
            Tax = request.Quantity * request.Price * 0.08m,
            CreatedAt = DateTime.UtcNow
        };
        _context.UserOrders.Add(order);
        await _context.SaveChangesAsync();
        await _emailService.SendAsync(request.Email, "Order confirmed", FormatReceipt(order));
        return CreatedAtAction(nameof(GetOrder), new { id = order.Id }, order);
    }

    [HttpPost("admins/orders")]
    public async Task<IActionResult> CreateAdminOrder([FromBody] AdminOrderRequest request)
    {
        if (string.IsNullOrWhiteSpace(request.Name) || string.IsNullOrWhiteSpace(request.Email))
            return BadRequest("Name and email are required");

        var normalizedName = request.Name.Trim().ToLowerInvariant();
        var order = new AdminOrder
        {
            Name = normalizedName,
            Email = request.Email,
            Amount = request.Quantity * request.Price,
            Tax = request.Quantity * request.Price * 0.08m,
            CreatedAt = DateTime.UtcNow
        };
        _context.AdminOrders.Add(order);
        await _context.SaveChangesAsync();
        await _emailService.SendAsync(request.Email, "Order confirmed", FormatReceipt(order));
        return CreatedAtAction(nameof(GetOrder), new { id = order.Id }, order);
    }
}
```

### Good Code (Fix)
```csharp
public class OrderController : ControllerBase
{
    private async Task<IActionResult> CreateOrder<T>(
        OrderRequest request, DbSet<T> dbSet) where T : BaseOrder, new()
    {
        if (string.IsNullOrWhiteSpace(request.Name) || string.IsNullOrWhiteSpace(request.Email))
            return BadRequest("Name and email are required");

        var normalizedName = request.Name.Trim().ToLowerInvariant();
        var order = new T
        {
            Name = normalizedName,
            Email = request.Email,
            Amount = request.Quantity * request.Price,
            Tax = request.Quantity * request.Price * 0.08m,
            CreatedAt = DateTime.UtcNow
        };
        dbSet.Add(order);
        await _context.SaveChangesAsync();
        await _emailService.SendAsync(request.Email, "Order confirmed", FormatReceipt(order));
        return CreatedAtAction(nameof(GetOrder), new { id = order.Id }, order);
    }

    [HttpPost("users/orders")]
    public Task<IActionResult> CreateUserOrder([FromBody] UserOrderRequest request)
        => CreateOrder(request, _context.UserOrders);

    [HttpPost("admins/orders")]
    public Task<IActionResult> CreateAdminOrder([FromBody] AdminOrderRequest request)
        => CreateOrder(request, _context.AdminOrders);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `block`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(method_declaration
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
The same sequence of 5+ statements repeated within a method or across methods in the same class, often due to copy-paste during development. Common in duplicated LINQ queries and repeated controller action patterns.

### Bad Code (Anti-pattern)
```csharp
public class ReportService
{
    public ReportResult GenerateSalesReport(DateTime startDate, DateTime endDate)
    {
        var records = _context.Sales
            .Where(s => s.Date >= startDate && s.Date <= endDate)
            .ToList();
        var filtered = records.Where(r => r.Amount > 0).ToList();
        var grouped = filtered.GroupBy(r => r.Date.Month)
            .Select(g => new MonthSummary
            {
                Month = g.Key,
                Total = g.Sum(r => r.Amount),
                Average = g.Average(r => r.Amount),
                Count = g.Count()
            }).ToList();
        var report = new ReportResult { Summaries = grouped, GeneratedAt = DateTime.UtcNow };
        _cache.Set($"sales_{startDate}_{endDate}", report, TimeSpan.FromMinutes(30));
        return report;
    }

    public ReportResult GenerateReturnsReport(DateTime startDate, DateTime endDate)
    {
        var records = _context.Returns
            .Where(s => s.Date >= startDate && s.Date <= endDate)
            .ToList();
        var filtered = records.Where(r => r.Amount > 0).ToList();
        var grouped = filtered.GroupBy(r => r.Date.Month)
            .Select(g => new MonthSummary
            {
                Month = g.Key,
                Total = g.Sum(r => r.Amount),
                Average = g.Average(r => r.Amount),
                Count = g.Count()
            }).ToList();
        var report = new ReportResult { Summaries = grouped, GeneratedAt = DateTime.UtcNow };
        _cache.Set($"returns_{startDate}_{endDate}", report, TimeSpan.FromMinutes(30));
        return report;
    }
}
```

### Good Code (Fix)
```csharp
public class ReportService
{
    private ReportResult BuildReport<T>(
        IQueryable<T> source, DateTime startDate, DateTime endDate, string cachePrefix)
        where T : IAmountRecord
    {
        var records = source
            .Where(s => s.Date >= startDate && s.Date <= endDate)
            .ToList();
        var filtered = records.Where(r => r.Amount > 0).ToList();
        var grouped = filtered.GroupBy(r => r.Date.Month)
            .Select(g => new MonthSummary
            {
                Month = g.Key,
                Total = g.Sum(r => r.Amount),
                Average = g.Average(r => r.Amount),
                Count = g.Count()
            }).ToList();
        var report = new ReportResult { Summaries = grouped, GeneratedAt = DateTime.UtcNow };
        _cache.Set($"{cachePrefix}_{startDate}_{endDate}", report, TimeSpan.FromMinutes(30));
        return report;
    }

    public ReportResult GenerateSalesReport(DateTime startDate, DateTime endDate)
        => BuildReport(_context.Sales, startDate, endDate, "sales");

    public ReportResult GenerateReturnsReport(DateTime startDate, DateTime endDate)
        => BuildReport(_context.Returns, startDate, endDate, "returns");
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `block`, `local_declaration_statement`, `expression_statement`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across method bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(block
  (_) @stmt)

(method_declaration
  body: (block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
