# Cognitive Complexity -- C#

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, catch, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cs`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/catch, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```csharp
public List<Result> ProcessOrders(List<Order> orders, Config config)
{
    var results = new List<Result>();
    foreach (var order in orders)
    {
        if (order.IsActive)
        {
            try
            {
                if (order.Type == OrderType.Premium)
                {
                    foreach (var item in order.Items)
                    {
                        if (config.RequiredSkus.Contains(item.Sku))
                        {
                            if (item.Quantity <= 0)
                            {
                                continue;
                            }
                            if (item.Price > config.MaxPrice)
                            {
                                break;
                            }
                            results.Add(TransformItem(item));
                        }
                    }
                }
                else
                {
                    if (order.Fallback != null)
                    {
                        results.Add(DefaultTransform(order));
                    }
                }
            }
            catch (ValidationException e)
            {
                if (config.Strict)
                {
                    throw;
                }
                results.Add(ErrorResult(order, e));
            }
        }
    }
    return results;
}
```

### Good Code (Fix)
```csharp
private List<Result> ProcessPremiumItems(IEnumerable<LineItem> items, Config config)
{
    var results = new List<Result>();
    foreach (var item in items)
    {
        if (!config.RequiredSkus.Contains(item.Sku)) continue;
        if (item.Price > config.MaxPrice) break;
        if (item.Quantity <= 0) continue;
        results.Add(TransformItem(item));
    }
    return results;
}

private Result? ProcessSingleOrder(Order order, Config config)
{
    if (!order.IsActive) return null;
    if (order.Type == OrderType.Premium)
        return CompositeResult(ProcessPremiumItems(order.Items, config));
    if (order.Fallback != null)
        return DefaultTransform(order);
    return null;
}

public List<Result> ProcessOrders(List<Order> orders, Config config)
{
    var results = new List<Result>();
    foreach (var order in orders)
    {
        try
        {
            var r = ProcessSingleOrder(order, config);
            if (r != null) results.Add(r);
        }
        catch (ValidationException e)
        {
            if (config.Strict) throw;
            results.Add(ErrorResult(order, e));
        }
    }
    return results;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `for_each_statement`, `while_statement`, `do_statement`, `try_statement`, `catch_clause`, `switch_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and `throw` statements add 1 each for flow disruption. `else` clauses and `catch` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find method boundaries
(method_declaration body: (block) @fn_body) @fn
(constructor_declaration body: (block) @fn_body) @fn
(local_function_statement body: (block) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(for_each_statement) @nesting
(while_statement) @nesting
(do_statement) @nesting
(try_statement) @nesting
(switch_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(throw_statement) @flow_break

;; Else/catch/finally break linear flow
(else_clause) @flow_break
(catch_clause) @flow_break
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
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/catch per operation that fragments the readable logic flow.

### Bad Code (Anti-pattern)
```csharp
public async Task<SyncResult> SyncUserDataAsync(string userId)
{
    User user;
    try
    {
        user = await FetchUserAsync(userId);
    }
    catch (HttpRequestException)
    {
        return SyncResult.Failure("fetch_failed");
    }

    Profile profile;
    try
    {
        profile = await FetchProfileAsync(user.ProfileId);
    }
    catch (HttpRequestException)
    {
        return SyncResult.Failure("profile_failed");
    }

    Preferences preferences;
    try
    {
        preferences = await LoadPreferencesAsync(user.Id);
    }
    catch (FileNotFoundException)
    {
        preferences = Preferences.Default;
    }

    MergedData merged;
    try
    {
        merged = MergeData(user, profile, preferences);
    }
    catch (InvalidOperationException)
    {
        return SyncResult.Failure("merge_failed");
    }

    try
    {
        await SaveToCacheAsync(merged);
    }
    catch (CacheException e)
    {
        _logger.LogWarning(e, "Cache save failed");
    }

    try
    {
        await NotifyServicesAsync(merged);
    }
    catch (NotificationException)
    {
        return SyncResult.Failure("notify_failed");
    }

    return SyncResult.Success(merged);
}
```

### Good Code (Fix)
```csharp
public async Task<SyncResult> SyncUserDataAsync(string userId)
{
    try
    {
        var user = await FetchUserAsync(userId);
        var profile = await FetchProfileAsync(user.ProfileId);
        var preferences = await LoadPreferencesSafeAsync(user.Id);
        var merged = MergeData(user, profile, preferences);

        await SaveToCacheSafeAsync(merged);
        await NotifyServicesAsync(merged);

        return SyncResult.Success(merged);
    }
    catch (HttpRequestException e)
    {
        return SyncResult.Failure(IdentifyStage(e));
    }
    catch (InvalidOperationException)
    {
        return SyncResult.Failure("merge_failed");
    }
    catch (NotificationException)
    {
        return SyncResult.Failure("notify_failed");
    }
}

private async Task<Preferences> LoadPreferencesSafeAsync(long userId)
{
    try { return await LoadPreferencesAsync(userId); }
    catch (FileNotFoundException) { return Preferences.Default; }
}

private async Task SaveToCacheSafeAsync(MergedData data)
{
    try { await SaveToCacheAsync(data); }
    catch (CacheException e) { _logger.LogWarning(e, "Cache save failed"); }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`
- **Detection approach**: Count `try_statement` nodes within a single method body. If 3 or more try/catch blocks appear as siblings (not nested), flag as interleaved error handling. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a method
(method_declaration
  body: (block
    (try_statement) @try1
    (try_statement) @try2
    (try_statement) @try3))

;; Detect catch clauses
(catch_clause) @error_handler
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
