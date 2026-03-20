# SSRF -- C#

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In C# / .NET applications, this commonly manifests through `HttpClient.GetAsync()`, `HttpClient.SendAsync()`, `WebClient.DownloadString()`, or `HttpWebRequest` receiving unsanitized URLs from query parameters, form data, or API payloads. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, Azure IMDS, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Frameworks/libraries**: System.Net.Http.HttpClient, System.Net.WebClient, System.Net.HttpWebRequest, RestSharp, ASP.NET Core, ASP.NET MVC

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to `HttpClient.GetAsync()`, `HttpClient.GetStringAsync()`, `WebClient.DownloadString()`, or similar HTTP client methods without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```csharp
using Microsoft.AspNetCore.Mvc;
using System.Net.Http;

[ApiController]
[Route("[controller]")]
public class ProxyController : ControllerBase
{
    private readonly HttpClient _httpClient;

    public ProxyController(HttpClient httpClient)
    {
        _httpClient = httpClient;
    }

    [HttpGet("fetch")]
    public async Task<IActionResult> Fetch([FromQuery] string url)
    {
        // User-controlled URL passed directly to HttpClient
        var response = await _httpClient.GetStringAsync(url);
        return Ok(response);
    }

    [HttpGet("proxy")]
    public async Task<IActionResult> Proxy([FromQuery] string targetUrl)
    {
        // No validation on the URL before making the request
        var response = await _httpClient.GetAsync(targetUrl);
        var content = await response.Content.ReadAsStringAsync();
        return Content(content);
    }
}
```

### Good Code (Fix)
```csharp
using Microsoft.AspNetCore.Mvc;
using System.Net;
using System.Net.Http;

[ApiController]
[Route("[controller]")]
public class ProxyController : ControllerBase
{
    private readonly HttpClient _httpClient;
    private static readonly HashSet<string> AllowedHosts = new()
    {
        "api.example.com",
        "cdn.example.com"
    };

    public ProxyController(HttpClient httpClient)
    {
        _httpClient = httpClient;
    }

    private static Uri ValidateUrl(string input)
    {
        if (!Uri.TryCreate(input, UriKind.Absolute, out var parsed))
            throw new ArgumentException("Invalid URL");

        if (parsed.Scheme != "http" && parsed.Scheme != "https")
            throw new ArgumentException("Only HTTP(S) URLs are allowed");

        if (!AllowedHosts.Contains(parsed.Host))
            throw new ArgumentException("Host not in allowlist");

        // Resolve DNS and check for private IP ranges
        var addresses = Dns.GetHostAddresses(parsed.Host);
        foreach (var addr in addresses)
        {
            if (IPAddress.IsLoopback(addr) || IsPrivateIp(addr))
                throw new ArgumentException("URL resolves to blocked IP range");
        }
        return parsed;
    }

    private static bool IsPrivateIp(IPAddress ip)
    {
        byte[] bytes = ip.GetAddressBytes();
        return bytes[0] switch
        {
            10 => true,
            172 => bytes[1] >= 16 && bytes[1] <= 31,
            192 => bytes[1] == 168,
            169 => bytes[1] == 254,
            127 => true,
            _ => false
        };
    }

    [HttpGet("fetch")]
    public async Task<IActionResult> Fetch([FromQuery] string url)
    {
        try
        {
            var validated = ValidateUrl(url);
            var response = await _httpClient.GetStringAsync(validated);
            return Ok(response);
        }
        catch (ArgumentException)
        {
            return BadRequest("Invalid URL");
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `member_access_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `invocation_expression` nodes where the method is `GetAsync`, `GetStringAsync`, `PostAsync`, `SendAsync` (on `HttpClient`), `DownloadString` (on `WebClient`), or `GetResponse` (on `HttpWebRequest`), and the URL argument is an `identifier` tracing to user-controlled input (e.g., `[FromQuery]` parameters, `Request.Query`). Flag cases where no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (member_access_expression
    expression: (identifier) @client_obj
    name: (identifier) @method)
  arguments: (argument_list
    (identifier) @url_arg))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `user_controlled_url_http_request`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Open Redirect via User-Controlled Redirect Target

### Description
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In ASP.NET applications, this typically occurs through `Redirect()`, `RedirectPermanent()`, or `RedirectToAction()` with user-controlled URL arguments. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```csharp
using Microsoft.AspNetCore.Mvc;

[ApiController]
[Route("[controller]")]
public class AuthController : ControllerBase
{
    [HttpGet("redirect")]
    public IActionResult HandleRedirect([FromQuery] string url)
    {
        // User-controlled redirect target -- no validation
        return Redirect(url);
    }

    [HttpGet("login-callback")]
    public IActionResult LoginCallback([FromQuery] string returnTo)
    {
        // No validation on redirect destination
        return Redirect(returnTo ?? "/");
    }
}
```

### Good Code (Fix)
```csharp
using Microsoft.AspNetCore.Mvc;

[ApiController]
[Route("[controller]")]
public class AuthController : ControllerBase
{
    private static readonly HashSet<string> AllowedRedirectDomains = new()
    {
        "example.com",
        "app.example.com"
    };

    private bool IsSafeRedirect(string target)
    {
        // Allow relative paths (same-origin)
        if (target.StartsWith("/") && !target.StartsWith("//"))
            return true;

        if (Uri.TryCreate(target, UriKind.Absolute, out var parsed))
            return AllowedRedirectDomains.Contains(parsed.Host);

        return false;
    }

    [HttpGet("redirect")]
    public IActionResult HandleRedirect([FromQuery] string url)
    {
        if (!IsSafeRedirect(url))
            return BadRequest("Invalid redirect target");

        return Redirect(url);
    }

    [HttpGet("login-callback")]
    public IActionResult LoginCallback([FromQuery] string returnTo)
    {
        if (string.IsNullOrEmpty(returnTo) || !IsSafeRedirect(returnTo))
            return Redirect("/");

        // ASP.NET also provides LocalRedirect() for same-origin redirects
        return Redirect(returnTo);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `invocation_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `invocation_expression` nodes where the method is `Redirect`, `RedirectPermanent`, or `RedirectToAction` and the argument is an `identifier` tracing to user-controlled input such as `[FromQuery]` parameters, `Request.Query`, or method parameters. Flag when no URL validation or `Url.IsLocalUrl()` check precedes the redirect call.
- **S-expression query sketch**:
```scheme
(invocation_expression
  function: (identifier) @method
  arguments: (argument_list
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
