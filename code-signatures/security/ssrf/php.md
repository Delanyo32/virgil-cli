# SSRF -- PHP

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In PHP applications, this commonly manifests through `file_get_contents()`, `curl_exec()`, or Guzzle HTTP client receiving unsanitized URLs from `$_GET`, `$_POST`, or API payloads. PHP's `file_get_contents()` is particularly dangerous because it supports multiple stream wrappers (`file://`, `php://`, `gopher://`, `dict://`) beyond HTTP, widening the attack surface.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. PHP's stream wrapper support makes SSRF especially dangerous -- `gopher://` can be used to interact with Redis, Memcached, and SMTP, and `file://` can read local files. SSRF is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: curl (ext-curl), file_get_contents, fopen, Guzzle, Symfony HttpClient, Laravel Http facade

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to `file_get_contents()`, `curl_setopt()` with `CURLOPT_URL`, `fopen()`, or Guzzle's `$client->get()` without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or leverage PHP stream wrappers for local file access and protocol interaction.

### Bad Code (Anti-pattern)
```php
<?php
// file_get_contents with user-controlled URL
$url = $_GET['url'];
$content = file_get_contents($url);
echo $content;

// cURL with user-controlled URL
function fetchUrl(string $userUrl): string {
    $ch = curl_init();
    curl_setopt($ch, CURLOPT_URL, $userUrl);
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    $response = curl_exec($ch);
    curl_close($ch);
    return $response;
}

echo fetchUrl($_POST['target']);
```

### Good Code (Fix)
```php
<?php
const ALLOWED_HOSTS = ['api.example.com', 'cdn.example.com'];
const ALLOWED_SCHEMES = ['http', 'https'];

function validateUrl(string $input): string {
    $parsed = parse_url($input);
    if ($parsed === false || !isset($parsed['scheme'], $parsed['host'])) {
        throw new InvalidArgumentException('Invalid URL');
    }
    if (!in_array($parsed['scheme'], ALLOWED_SCHEMES, true)) {
        throw new InvalidArgumentException('Only HTTP(S) URLs are allowed');
    }
    if (!in_array($parsed['host'], ALLOWED_HOSTS, true)) {
        throw new InvalidArgumentException('Host not in allowlist');
    }
    // Resolve DNS and check for private IP ranges
    $ip = gethostbyname($parsed['host']);
    if ($ip === $parsed['host']) {
        throw new InvalidArgumentException('Cannot resolve hostname');
    }
    if (filter_var($ip, FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE | FILTER_FLAG_NO_RES_RANGE) === false) {
        throw new InvalidArgumentException('URL resolves to blocked IP range');
    }
    return $input;
}

try {
    $validatedUrl = validateUrl($_GET['url']);
    $ch = curl_init();
    curl_setopt($ch, CURLOPT_URL, $validatedUrl);
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ch, CURLOPT_FOLLOWLOCATION, false); // Prevent redirect-based SSRF
    curl_setopt($ch, CURLOPT_PROTOCOLS, CURLPROTO_HTTP | CURLPROTO_HTTPS); // Block non-HTTP protocols
    $response = curl_exec($ch);
    curl_close($ch);
    echo $response;
} catch (InvalidArgumentException $e) {
    http_response_code(400);
    echo 'Invalid URL';
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `argument`, `variable_name`, `member_access_expression`
- **Detection approach**: Find `function_call_expression` nodes where the function name is `file_get_contents`, `fopen`, `curl_setopt` (with `CURLOPT_URL` constant), or Guzzle method calls like `$client->get()`, and the URL argument is a `variable_name` tracing to user-controlled input (`$_GET`, `$_POST`, `$_REQUEST`, function parameters). Flag cases where no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (variable_name) @url_arg)))

(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (name) @constant)
    (argument
      (variable_name) @url_arg)))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `user_controlled_url_http_request`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Open Redirect via User-Controlled Redirect Target

### Description
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In PHP applications, this typically occurs through `header('Location: ...')` with user-controlled values, or via framework redirect helpers. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```php
<?php
// header() with user-controlled redirect target
$target = $_GET['url'];
header("Location: $target");
exit;

// Laravel example
use Illuminate\Http\Request;

Route::get('/login-callback', function (Request $request) {
    $returnTo = $request->input('next', '/');
    // No validation on redirect destination
    return redirect($returnTo);
});
```

### Good Code (Fix)
```php
<?php
const ALLOWED_REDIRECT_DOMAINS = ['example.com', 'app.example.com'];

function isSafeRedirect(string $target): bool {
    // Allow relative paths (same-origin)
    if (str_starts_with($target, '/') && !str_starts_with($target, '//')) {
        return true;
    }
    $parsed = parse_url($target);
    if ($parsed === false || !isset($parsed['host'])) {
        return false;
    }
    return in_array($parsed['host'], ALLOWED_REDIRECT_DOMAINS, true);
}

$target = $_GET['url'] ?? '/';
if (!isSafeRedirect($target)) {
    http_response_code(400);
    echo 'Invalid redirect target';
    exit;
}
header("Location: $target");
exit;

// Laravel example with validation
use Illuminate\Http\Request;

Route::get('/login-callback', function (Request $request) {
    $returnTo = $request->input('next', '/');
    if (!isSafeRedirect($returnTo)) {
        return redirect('/');
    }
    return redirect($returnTo);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `argument`, `encapsed_string`, `variable_name`
- **Detection approach**: Find `function_call_expression` nodes where the function is `header` and the argument contains a `Location:` string with an embedded `variable_name` (interpolation), or where the function is `redirect` (Laravel/Symfony) with a `variable_name` argument tracing to user input. Flag when no URL validation precedes the redirect.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (encapsed_string
        (variable_name) @redirect_target))))

(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (variable_name) @redirect_target)))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
