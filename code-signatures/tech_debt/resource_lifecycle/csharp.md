# Resource Lifecycle -- C#

## Overview
Resources that are acquired but never properly released cause memory leaks, handle exhaustion, and socket pool starvation. In C#, the most common manifestations are `IDisposable` objects not wrapped in `using` statements and `HttpClient` instances created per-request instead of being reused.

## Why It's a Tech Debt Concern
C#'s `IDisposable` pattern and `using` statement provide deterministic resource cleanup, but developers frequently bypass them by calling `Dispose()` manually (which is skipped on exceptions) or not calling it at all. The garbage collector's finalizer will eventually run, but finalization is non-deterministic and adds GC pressure. `HttpClient` created per-request is an especially insidious anti-pattern because disposing `HttpClient` does not immediately release the underlying socket -- it enters a `TIME_WAIT` state for up to 240 seconds, leading to socket exhaustion under load even though the code appears correct.

## Applicability
- **Relevance**: high (file I/O, database connections, HTTP clients)
- **Languages covered**: `.cs`
- **Frameworks/libraries**: ASP.NET Core, Entity Framework, ADO.NET, System.Net.Http

---

## Pattern 1: IDisposable Not Wrapped in using Statement

### Description
Creating an object that implements `IDisposable` (such as `StreamReader`, `SqlConnection`, `FileStream`, `MemoryStream`) and assigning it to a local variable without wrapping it in a `using` statement or `using` declaration. If an exception occurs between construction and the manual `.Dispose()` call, the resource leaks.

### Bad Code (Anti-pattern)
```csharp
// StreamReader not in using -- leaked if ReadToEnd() throws
public string ReadFile(string path)
{
    var reader = new StreamReader(path);
    string content = reader.ReadToEnd();
    reader.Dispose(); // Never reached if ReadToEnd() throws
    return content;
}

// SqlConnection manually closed but not on error path
public List<User> GetUsers(string connString)
{
    var conn = new SqlConnection(connString);
    conn.Open();
    var cmd = new SqlCommand("SELECT * FROM Users", conn);
    var reader = cmd.ExecuteReader();
    var users = new List<User>();
    while (reader.Read())
    {
        users.Add(new User(reader.GetInt32(0), reader.GetString(1)));
    }
    reader.Close();
    conn.Close(); // Leaked if any line above throws
    return users;
}

// MemoryStream created but never disposed
public byte[] CompressData(byte[] input)
{
    var output = new MemoryStream();
    var gzip = new GZipStream(output, CompressionMode.Compress);
    gzip.Write(input, 0, input.Length);
    gzip.Close();
    return output.ToArray();
    // output never disposed
}
```

### Good Code (Fix)
```csharp
// using statement ensures Dispose on all paths
public string ReadFile(string path)
{
    using var reader = new StreamReader(path);
    return reader.ReadToEnd();
}

// All IDisposable resources in using statements
public List<User> GetUsers(string connString)
{
    using var conn = new SqlConnection(connString);
    conn.Open();
    using var cmd = new SqlCommand("SELECT * FROM Users", conn);
    using var reader = cmd.ExecuteReader();
    var users = new List<User>();
    while (reader.Read())
    {
        users.Add(new User(reader.GetInt32(0), reader.GetString(1)));
    }
    return users;
}

// Both streams in using declarations
public byte[] CompressData(byte[] input)
{
    using var output = new MemoryStream();
    using var gzip = new GZipStream(output, CompressionMode.Compress);
    gzip.Write(input, 0, input.Length);
    gzip.Flush();
    return output.ToArray();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `local_declaration_statement`, `variable_declaration`, `object_creation_expression`, `using_statement`
- **Detection approach**: Find `object_creation_expression` nodes creating instances of known `IDisposable` types (`StreamReader`, `SqlConnection`, `FileStream`, `MemoryStream`, `HttpClient`, etc.) assigned in a `local_declaration_statement`. Check if the statement is a `using_statement` or has the `using` modifier (C# 8 `using` declaration). Flag declarations of disposable types that lack the `using` keyword.
- **S-expression query sketch**:
  ```scheme
  ;; Disposable creation without using
  (local_declaration_statement
    (variable_declaration
      type: (_) @type
      (variable_declarator
        (identifier) @var_name
        (equals_value_clause
          (object_creation_expression
            type: (identifier) @created_type)))))

  ;; Safe: using declaration
  (using_statement
    (variable_declaration
      (variable_declarator
        (identifier) @var_name
        (equals_value_clause
          (object_creation_expression
            type: (identifier) @created_type)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `disposable_not_disposed`
- **Pattern name**: `idisposable_without_using`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: HttpClient Created Per-Request

### Description
Creating a new `HttpClient` instance for each HTTP request, typically inside a request handler, loop, or method call. `HttpClient` is designed to be long-lived and reused. Each disposal leaves sockets in `TIME_WAIT` state for up to 4 minutes, and creating many instances exhausts the socket pool. Additionally, new instances do not respect DNS changes because they cache DNS resolution at construction time.

### Bad Code (Anti-pattern)
```csharp
// New HttpClient per request -- socket exhaustion under load
public async Task<string> GetUserData(int userId)
{
    var client = new HttpClient();
    var response = await client.GetAsync($"https://api.example.com/users/{userId}");
    return await response.Content.ReadAsStringAsync();
    // client disposed by GC eventually, socket in TIME_WAIT
}

// HttpClient in a loop
public async Task<List<string>> FetchAll(IEnumerable<string> urls)
{
    var results = new List<string>();
    foreach (var url in urls)
    {
        using var client = new HttpClient(); // New client per URL
        var response = await client.GetAsync(url);
        results.Add(await response.Content.ReadAsStringAsync());
    }
    return results;
}

// HttpClient created in controller action
[HttpGet("{id}")]
public async Task<IActionResult> Get(int id)
{
    var client = new HttpClient();
    client.BaseAddress = new Uri("https://api.example.com");
    var result = await client.GetStringAsync($"/items/{id}");
    return Ok(result);
}
```

### Good Code (Fix)
```csharp
// Inject IHttpClientFactory (ASP.NET Core recommended pattern)
public class UserService
{
    private readonly IHttpClientFactory _clientFactory;

    public UserService(IHttpClientFactory clientFactory)
    {
        _clientFactory = clientFactory;
    }

    public async Task<string> GetUserData(int userId)
    {
        var client = _clientFactory.CreateClient();
        var response = await client.GetAsync($"https://api.example.com/users/{userId}");
        return await response.Content.ReadAsStringAsync();
    }
}

// Static/shared HttpClient for simple cases
public static class ApiClient
{
    private static readonly HttpClient _client = new HttpClient
    {
        BaseAddress = new Uri("https://api.example.com"),
        Timeout = TimeSpan.FromSeconds(30)
    };

    public static async Task<List<string>> FetchAll(IEnumerable<string> urls)
    {
        var results = new List<string>();
        foreach (var url in urls)
        {
            var response = await _client.GetAsync(url);
            results.Add(await response.Content.ReadAsStringAsync());
        }
        return results;
    }
}

// Controller using IHttpClientFactory
[HttpGet("{id}")]
public async Task<IActionResult> Get(int id,
    [FromServices] IHttpClientFactory clientFactory)
{
    var client = clientFactory.CreateClient("ExampleApi");
    var result = await client.GetStringAsync($"/items/{id}");
    return Ok(result);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `identifier`, `local_declaration_statement`
- **Detection approach**: Find `object_creation_expression` nodes where the type is `HttpClient` that appear inside method bodies (not static field initializers). Check if the creation is inside a `foreach_statement`, `for_statement`, or a method that looks like a request handler (ASP.NET `[HttpGet]` attribute). Flag any local `new HttpClient()` that is not in a static field or constructor.
- **S-expression query sketch**:
  ```scheme
  ;; HttpClient created locally in a method
  (local_declaration_statement
    (variable_declaration
      (variable_declarator
        (identifier) @var_name
        (equals_value_clause
          (object_creation_expression
            type: (identifier) @type_name)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `disposable_not_disposed`
- **Pattern name**: `httpclient_per_request`
- **Severity**: warning
- **Confidence**: high
