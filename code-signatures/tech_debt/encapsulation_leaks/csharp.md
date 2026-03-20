# Encapsulation Leaks -- C#

## Overview
Encapsulation leaks in C# occur when mutable static fields create shared global state that persists across requests or threads, or when configuration values are hardcoded directly into class logic instead of being injected through `IOptions<T>`, `IConfiguration`, or constructor parameters. Both patterns tightly couple code to runtime conditions and make testing and deployment across environments difficult.

## Why It's a Tech Debt Concern
Mutable static fields are shared across all threads and requests in ASP.NET Core applications, creating race conditions and making behavior depend on request ordering. Static state cannot be reset between tests, causing flaky test suites with order-dependent failures. Hardcoded configuration values require code changes and redeployment for every environment difference (dev, staging, production), violate the twelve-factor app methodology, and make it impossible to override settings without modifying source.

## Applicability
- **Relevance**: high (static state and hardcoded config are common in C# enterprise codebases)
- **Languages covered**: `.cs`
- **Frameworks/libraries**: ASP.NET Core (request-scoped vs static), Entity Framework (DbContext lifetime), Azure Functions (static state across invocations)

---

## Pattern 1: Static Global State

### Description
A class uses `static` mutable fields (non-`readonly`, non-`const`) to store state that is read and modified at runtime. In server applications, this state is shared across all requests and threads, creating race conditions and making behavior unpredictable.

### Bad Code (Anti-pattern)
```csharp
public class RateLimiter
{
    private static Dictionary<string, int> _requestCounts = new();
    private static DateTime _windowStart = DateTime.UtcNow;
    private static int _globalRequestCount = 0;
    public static bool IsEnabled = true;

    public bool AllowRequest(string clientId)
    {
        if (!IsEnabled) return true;

        _globalRequestCount++;  // race condition

        if (DateTime.UtcNow - _windowStart > TimeSpan.FromMinutes(1))
        {
            _requestCounts.Clear();  // race condition
            _windowStart = DateTime.UtcNow;
        }

        if (!_requestCounts.ContainsKey(clientId))
            _requestCounts[clientId] = 0;

        _requestCounts[clientId]++;  // race condition

        return _requestCounts[clientId] <= 100;
    }
}

public static class AppState
{
    public static List<string> ActiveUsers = new();
    public static int TotalProcessed = 0;
    public static string LastError = null;
}

// Any code can mutate shared state
AppState.ActiveUsers.Add("user1");  // not thread-safe
AppState.TotalProcessed++;          // race condition
RateLimiter.IsEnabled = false;      // disables globally
```

### Good Code (Fix)
```csharp
public class RateLimiter : IRateLimiter
{
    private readonly ConcurrentDictionary<string, int> _requestCounts = new();
    private DateTime _windowStart = DateTime.UtcNow;
    private int _globalRequestCount;
    private readonly bool _enabled;
    private readonly object _lock = new();

    public RateLimiter(IOptions<RateLimiterOptions> options)
    {
        _enabled = options.Value.Enabled;
    }

    public bool AllowRequest(string clientId)
    {
        if (!_enabled) return true;

        Interlocked.Increment(ref _globalRequestCount);

        lock (_lock)
        {
            if (DateTime.UtcNow - _windowStart > TimeSpan.FromMinutes(1))
            {
                _requestCounts.Clear();
                _windowStart = DateTime.UtcNow;
            }
        }

        var count = _requestCounts.AddOrUpdate(clientId, 1, (_, c) => c + 1);
        return count <= 100;
    }
}

// Registered as scoped or singleton via DI
services.AddSingleton<IRateLimiter, RateLimiter>();
services.Configure<RateLimiterOptions>(configuration.GetSection("RateLimiter"));
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` inside `class_declaration` with `static` modifier, excluding `readonly` and `const`
- **Detection approach**: Find `field_declaration` nodes inside `class_declaration` whose `modifier` children include `static` but do not include `readonly` or `const`. These are mutable static fields. Flag classes with 1+ mutable static fields. Stronger signal when the field type is a collection (`Dictionary`, `List`, `HashSet`) or a primitive that gets incremented.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (declaration_list
      (field_declaration
        (modifier) @mod
        type: (_) @field_type
        (variable_declaration
          (variable_declarator
            (identifier) @field_name)))))
  ```

### Pipeline Mapping
- **Pipeline name**: `static_global_state`
- **Pattern name**: `mutable_static_field`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hardcoded Configuration

### Description
Classes embed configuration values (connection strings, URLs, timeouts, feature flags, API keys) as string literals, magic numbers, or compile-time constants instead of reading them from injected configuration. This requires code changes to adjust behavior across environments and prevents runtime reconfiguration.

### Bad Code (Anti-pattern)
```csharp
public class EmailService
{
    public async Task SendAsync(string to, string subject, string body)
    {
        using var client = new SmtpClient("smtp.company.com", 587);
        client.Credentials = new NetworkCredential("noreply@company.com", "s3cretP@ss!");
        client.EnableSsl = true;

        var message = new MailMessage("noreply@company.com", to, subject, body);
        await client.SendMailAsync(message);
    }
}

public class PaymentGateway
{
    private const string ApiUrl = "https://api.stripe.com/v1";
    private const string ApiKey = "sk_live_abc123xyz";
    private const int TimeoutSeconds = 30;
    private const int MaxRetries = 3;

    public async Task<PaymentResult> ChargeAsync(decimal amount, string currency)
    {
        var client = new HttpClient { BaseAddress = new Uri(ApiUrl) };
        client.Timeout = TimeSpan.FromSeconds(TimeoutSeconds);
        client.DefaultRequestHeaders.Add("Authorization", $"Bearer {ApiKey}");
        // ...
    }
}

public class CacheService
{
    public IDatabase GetDatabase()
    {
        var redis = ConnectionMultiplexer.Connect("redis-prod.internal:6379,password=r3d1s!");
        return redis.GetDatabase(0);
    }
}
```

### Good Code (Fix)
```csharp
public class EmailOptions
{
    public string SmtpHost { get; set; }
    public int SmtpPort { get; set; }
    public string FromAddress { get; set; }
    public string Username { get; set; }
    public string Password { get; set; }
    public bool UseSsl { get; set; }
}

public class EmailService
{
    private readonly EmailOptions _options;

    public EmailService(IOptions<EmailOptions> options)
    {
        _options = options.Value;
    }

    public async Task SendAsync(string to, string subject, string body)
    {
        using var client = new SmtpClient(_options.SmtpHost, _options.SmtpPort);
        client.Credentials = new NetworkCredential(_options.Username, _options.Password);
        client.EnableSsl = _options.UseSsl;

        var message = new MailMessage(_options.FromAddress, to, subject, body);
        await client.SendMailAsync(message);
    }
}

public class PaymentOptions
{
    public string ApiUrl { get; set; }
    public string ApiKey { get; set; }
    public int TimeoutSeconds { get; set; } = 30;
    public int MaxRetries { get; set; } = 3;
}

public class PaymentGateway
{
    private readonly PaymentOptions _options;
    private readonly HttpClient _client;

    public PaymentGateway(IOptions<PaymentOptions> options, HttpClient client)
    {
        _options = options.Value;
        _client = client;
        _client.BaseAddress = new Uri(_options.ApiUrl);
        _client.Timeout = TimeSpan.FromSeconds(_options.TimeoutSeconds);
        _client.DefaultRequestHeaders.Add("Authorization", $"Bearer {_options.ApiKey}");
    }

    public async Task<PaymentResult> ChargeAsync(decimal amount, string currency) { /* ... */ }
}

// appsettings.json
// { "Email": { "SmtpHost": "smtp.company.com", ... }, "Payment": { "ApiUrl": "...", ... } }

// Startup.cs
services.Configure<EmailOptions>(configuration.GetSection("Email"));
services.Configure<PaymentOptions>(configuration.GetSection("Payment"));
services.AddTransient<EmailService>();
services.AddTransient<PaymentGateway>();
```

### Tree-sitter Detection Strategy
- **Target node types**: `field_declaration` with `const` or string literal assignments, constructor bodies with hardcoded `string_literal` arguments
- **Detection approach**: Find `field_declaration` nodes with string literal initializers that match URL patterns (`https?://`, connection strings, email addresses) or look like secrets (containing `password`, `key`, `secret`, `token`). Also detect `object_creation_expression` calls in method bodies where string literal arguments resemble hostnames, ports, or credentials. Flag classes with 2+ hardcoded configuration-like string literals.
- **S-expression query sketch**:
  ```scheme
  (field_declaration
    (variable_declaration
      (variable_declarator
        (identifier) @field_name
        (equals_value_clause
          (string_literal) @hardcoded_value))))

  (object_creation_expression
    type: (identifier) @type_name
    arguments: (argument_list
      (argument
        (string_literal) @hardcoded_arg)))
  ```

### Pipeline Mapping
- **Pipeline name**: `hardcoded_config`
- **Pattern name**: `hardcoded_configuration_value`
- **Severity**: warning
- **Confidence**: medium
