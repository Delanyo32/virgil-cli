# N+1 Queries -- C#

## Overview
N+1 query patterns in C# occur when Entity Framework LINQ queries, Dapper calls, or ADO.NET commands are executed inside loops instead of using eager loading (`Include`), batch queries, or `IN` clauses.

## Why It's a Scalability Concern
Each iteration opens a command against the DbContext or connection, generating a separate SQL round-trip. EF's lazy loading proxies make this especially deceptive — accessing a navigation property in a loop triggers transparent queries. Under concurrent ASP.NET Core requests, this saturates the connection pool and causes thread-pool starvation.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: Entity Framework Core, Dapper, ADO.NET, NHibernate

---

## Pattern 1: EF Core Find/FirstOrDefault in Loop

### Description
Calling `context.Set<T>().Find()`, `.FirstOrDefault()`, or `.Single()` inside a `foreach` or `for` loop instead of using `.Where(x => ids.Contains(x.Id))` with `Include()`.

### Bad Code (Anti-pattern)
```csharp
public List<OrderDto> GetOrderDetails(List<int> orderIds)
{
    var results = new List<OrderDto>();
    foreach (var id in orderIds)
    {
        var order = context.Orders.Find(id);
        var customer = context.Customers.FirstOrDefault(c => c.Id == order.CustomerId);
        results.Add(new OrderDto(order, customer));
    }
    return results;
}
```

### Good Code (Fix)
```csharp
public List<OrderDto> GetOrderDetails(List<int> orderIds)
{
    var orders = context.Orders
        .Where(o => orderIds.Contains(o.Id))
        .Include(o => o.Customer)
        .ToList();
    return orders.Select(o => new OrderDto(o, o.Customer)).ToList();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `for_statement`, `while_statement`, `invocation_expression`, `member_access_expression`
- **Detection approach**: Find `invocation_expression` nodes calling `Find`, `FirstOrDefault`, `SingleOrDefault`, `First`, `Single` via `member_access_expression`, nested inside a `foreach_statement` or `for_statement` body.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (block
    (local_declaration_statement
      (variable_declaration
        (variable_declarator
          (equals_value_clause
            (invocation_expression
              (member_access_expression
                name: (identifier) @method))))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `ef_find_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Dapper Query in Loop

### Description
Calling `connection.Query<T>()`, `connection.QueryFirst<T>()`, or `connection.ExecuteAsync()` inside a loop instead of using a single parameterized batch query.

### Bad Code (Anti-pattern)
```csharp
public async Task<List<User>> GetUsersByIds(IDbConnection conn, List<int> userIds)
{
    var users = new List<User>();
    foreach (var id in userIds)
    {
        var user = await conn.QueryFirstAsync<User>("SELECT * FROM Users WHERE Id = @Id", new { Id = id });
        users.Add(user);
    }
    return users;
}
```

### Good Code (Fix)
```csharp
public async Task<List<User>> GetUsersByIds(IDbConnection conn, List<int> userIds)
{
    var users = (await conn.QueryAsync<User>(
        "SELECT * FROM Users WHERE Id IN @Ids", new { Ids = userIds })).ToList();
    return users;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `invocation_expression`, `member_access_expression`, `await_expression`
- **Detection approach**: Find `invocation_expression` calling `Query`, `QueryFirst`, `QueryFirstAsync`, `QueryAsync`, `Execute`, `ExecuteAsync` via `member_access_expression` on a connection-like variable, inside a loop.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (block
    (local_declaration_statement
      (variable_declaration
        (variable_declarator
          (equals_value_clause
            (await_expression
              (invocation_expression
                (member_access_expression
                  name: (identifier) @method)))))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `dapper_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: ADO.NET SqlCommand in Loop

### Description
Creating and executing `SqlCommand.ExecuteReader()` or `ExecuteScalar()` inside a loop instead of building a single query with multiple parameters or using table-valued parameters.

### Bad Code (Anti-pattern)
```csharp
public List<Product> GetProducts(SqlConnection conn, List<int> productIds)
{
    var products = new List<Product>();
    foreach (var id in productIds)
    {
        using var cmd = new SqlCommand("SELECT * FROM Products WHERE Id = @Id", conn);
        cmd.Parameters.AddWithValue("@Id", id);
        using var reader = cmd.ExecuteReader();
        while (reader.Read())
        {
            products.Add(MapProduct(reader));
        }
    }
    return products;
}
```

### Good Code (Fix)
```csharp
public List<Product> GetProducts(SqlConnection conn, List<int> productIds)
{
    var paramNames = productIds.Select((_, i) => $"@p{i}").ToList();
    var sql = $"SELECT * FROM Products WHERE Id IN ({string.Join(",", paramNames)})";
    using var cmd = new SqlCommand(sql, conn);
    for (int i = 0; i < productIds.Count; i++)
    {
        cmd.Parameters.AddWithValue(paramNames[i], productIds[i]);
    }
    using var reader = cmd.ExecuteReader();
    var products = new List<Product>();
    while (reader.Read())
    {
        products.Add(MapProduct(reader));
    }
    return products;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `foreach_statement`, `for_statement`, `invocation_expression`, `member_access_expression`
- **Detection approach**: Find `invocation_expression` calling `ExecuteReader`, `ExecuteScalar`, `ExecuteNonQuery` via `member_access_expression`, nested inside a loop construct.
- **S-expression query sketch**:
```scheme
(foreach_statement
  body: (block
    (local_declaration_statement
      (variable_declaration
        (variable_declarator
          (equals_value_clause
            (invocation_expression
              (member_access_expression
                name: (identifier) @method))))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `ado_execute_in_loop`
- **Severity**: warning
- **Confidence**: high
