# Concurrency Misuse -- C#

## Overview
C# provides a rich async/await model through the Task Parallel Library (TPL), but misuse manifests as synchronously blocking on async operations (`.Result`, `.Wait()`, `.GetAwaiter().GetResult()`) and failing to propagate `CancellationToken` through async call chains. These patterns cause deadlocks, thread pool starvation, and unresponsive applications.

## Why It's a Tech Debt Concern
Sync-over-async (`.Result`/`.Wait()`) causes deadlocks in UI and ASP.NET contexts where the synchronization context is captured, and thread pool starvation in server scenarios where blocking threads cannot service other requests. Missing `CancellationToken` propagation means that when a user navigates away, an HTTP request times out, or an application shuts down, downstream operations continue running indefinitely — wasting CPU, holding database connections, and delaying shutdown.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`
- **Frameworks/libraries**: ASP.NET Core, Entity Framework, HttpClient, MediatR, gRPC
- **Existing pipeline**: `sync_over_async` in `src/audit/pipelines/csharp/` — extends with detection patterns
- **Existing pipeline**: `missing_cancellation_token` in `src/audit/pipelines/csharp/` — extends with detection patterns

---

## Pattern 1: Sync-over-Async

### Description
Calling `.Result`, `.Wait()`, or `.GetAwaiter().GetResult()` on a `Task` or `Task<T>` to synchronously block until the async operation completes. In ASP.NET and UI contexts with a `SynchronizationContext`, this causes deadlocks because the continuation needs to resume on the captured context, which is blocked by the `.Result`/`.Wait()` call. In server scenarios, it blocks a thread pool thread.

### Bad Code (Anti-pattern)
```csharp
public class UserService
{
    private readonly HttpClient _httpClient;

    public UserDto GetUser(int id)
    {
        // Deadlock in ASP.NET: .Result blocks the request thread,
        // and the continuation needs that same thread to complete
        var response = _httpClient.GetAsync($"/api/users/{id}").Result;
        var content = response.Content.ReadAsStringAsync().Result;
        return JsonSerializer.Deserialize<UserDto>(content);
    }

    public void SyncMethod()
    {
        // .Wait() has the same deadlock problem
        InitializeAsync().Wait();

        // .GetAwaiter().GetResult() avoids AggregateException wrapping
        // but still deadlocks
        var data = LoadDataAsync().GetAwaiter().GetResult();
    }
}
```

### Good Code (Fix)
```csharp
public class UserService
{
    private readonly HttpClient _httpClient;

    public async Task<UserDto> GetUserAsync(int id)
    {
        var response = await _httpClient.GetAsync($"/api/users/{id}");
        var content = await response.Content.ReadAsStringAsync();
        return JsonSerializer.Deserialize<UserDto>(content);
    }

    public async Task InitializeAndLoadAsync()
    {
        await InitializeAsync();
        var data = await LoadDataAsync();
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `member_access_expression`, `invocation_expression`, `identifier`
- **Detection approach**: Find `member_access_expression` nodes where the member name is `Result` and the object expression is a method call returning `Task`/`Task<T>`. Also find `invocation_expression` nodes where the method is `Wait` or `GetResult` on a task-returning expression. Exclude cases inside `Main()` methods or console app entry points where sync-over-async is sometimes unavoidable.
- **S-expression query sketch**:
```scheme
(member_access_expression
  expression: (invocation_expression) @async_call
  name: (identifier) @result_access)

(invocation_expression
  expression: (member_access_expression
    name: (identifier) @wait_method))
```

### Pipeline Mapping
- **Pipeline name**: `sync_over_async`
- **Pattern name**: `result_or_wait_on_task`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Missing CancellationToken Propagation

### Description
An async method accepts a `CancellationToken` parameter but does not pass it to downstream async calls (`HttpClient.GetAsync`, `DbContext.SaveChangesAsync`, `Task.Delay`, etc.), or a public async API does not accept a `CancellationToken` at all. This breaks the cancellation chain, causing work to continue after the caller has cancelled.

### Bad Code (Anti-pattern)
```csharp
public class OrderService
{
    private readonly HttpClient _httpClient;
    private readonly AppDbContext _dbContext;

    // Accepts CancellationToken but never passes it downstream
    public async Task<Order> CreateOrderAsync(OrderRequest request, CancellationToken cancellationToken)
    {
        // Missing cancellationToken parameter
        var inventory = await _httpClient.GetAsync($"/api/inventory/{request.ProductId}");
        var content = await inventory.Content.ReadAsStringAsync();

        var order = new Order { ProductId = request.ProductId, Quantity = request.Quantity };
        _dbContext.Orders.Add(order);

        // Missing cancellationToken parameter
        await _dbContext.SaveChangesAsync();

        // Using CancellationToken.None defeats the purpose
        await NotifyWarehouseAsync(order, CancellationToken.None);

        return order;
    }

    // Public async method without CancellationToken overload
    public async Task<List<Order>> GetOrdersAsync(string userId)
    {
        return await _dbContext.Orders
            .Where(o => o.UserId == userId)
            .ToListAsync();  // No CancellationToken
    }
}
```

### Good Code (Fix)
```csharp
public class OrderService
{
    private readonly HttpClient _httpClient;
    private readonly AppDbContext _dbContext;

    public async Task<Order> CreateOrderAsync(OrderRequest request, CancellationToken cancellationToken)
    {
        var inventory = await _httpClient.GetAsync(
            $"/api/inventory/{request.ProductId}", cancellationToken);
        var content = await inventory.Content.ReadAsStringAsync(cancellationToken);

        var order = new Order { ProductId = request.ProductId, Quantity = request.Quantity };
        _dbContext.Orders.Add(order);
        await _dbContext.SaveChangesAsync(cancellationToken);
        await NotifyWarehouseAsync(order, cancellationToken);

        return order;
    }

    public async Task<List<Order>> GetOrdersAsync(
        string userId, CancellationToken cancellationToken = default)
    {
        return await _dbContext.Orders
            .Where(o => o.UserId == userId)
            .ToListAsync(cancellationToken);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `parameter_list`, `invocation_expression`, `argument_list`
- **Detection approach**: Find `method_declaration` nodes with `async` modifier and a `CancellationToken` parameter. Within the body, find `invocation_expression` nodes calling known async methods (`GetAsync`, `PostAsync`, `SaveChangesAsync`, `ToListAsync`, `Delay`, etc.). Flag when the `argument_list` of those calls does not include the `CancellationToken` parameter name. Also flag when `CancellationToken.None` is passed instead of the method's token parameter.
- **S-expression query sketch**:
```scheme
(method_declaration
  parameters: (parameter_list
    (parameter
      type: (identifier) @param_type
      name: (identifier) @param_name))
  body: (block
    (expression_statement
      (await_expression
        (invocation_expression
          arguments: (argument_list) @args)))))
```

### Pipeline Mapping
- **Pipeline name**: `missing_cancellation_token`
- **Pattern name**: `token_not_forwarded`
- **Severity**: warning
- **Confidence**: high
