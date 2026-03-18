# SSRF -- Java

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In Java applications, this commonly manifests through `java.net.URL.openConnection()`, `HttpURLConnection`, `HttpClient.send()`, or libraries like Apache HttpClient and OkHttp receiving unsanitized URLs from request parameters, form data, or API payloads. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: java.net.URL, java.net.http.HttpClient, Apache HttpClient, OkHttp, Spring RestTemplate, Spring WebClient, Spring Boot

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to `new URL(userInput).openConnection()`, `HttpClient.send()`, `RestTemplate.getForObject()`, or similar HTTP client methods without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```java
import javax.servlet.http.*;
import java.net.*;
import java.io.*;
import java.net.http.*;

@WebServlet("/proxy")
public class ProxyServlet extends HttpServlet {
    @Override
    protected void doGet(HttpServletRequest req, HttpServletResponse resp) throws Exception {
        String targetUrl = req.getParameter("url");
        // User-controlled URL passed directly to URL.openConnection()
        URL url = new URL(targetUrl);
        HttpURLConnection conn = (HttpURLConnection) url.openConnection();
        InputStream in = conn.getInputStream();
        in.transferTo(resp.getOutputStream());
    }
}

// Java 11+ HttpClient example
public class FetchService {
    public String fetch(String userUrl) throws Exception {
        // User-controlled URL with no validation
        HttpClient client = HttpClient.newHttpClient();
        HttpRequest request = HttpRequest.newBuilder()
            .uri(URI.create(userUrl))
            .build();
        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());
        return response.body();
    }
}
```

### Good Code (Fix)
```java
import javax.servlet.http.*;
import java.net.*;
import java.io.*;
import java.util.Set;

@WebServlet("/proxy")
public class ProxyServlet extends HttpServlet {
    private static final Set<String> ALLOWED_HOSTS = Set.of("api.example.com", "cdn.example.com");
    private static final Set<String> ALLOWED_SCHEMES = Set.of("http", "https");

    private URL validateUrl(String input) throws Exception {
        URL parsed = new URL(input);
        if (!ALLOWED_SCHEMES.contains(parsed.getProtocol())) {
            throw new SecurityException("Only HTTP(S) URLs are allowed");
        }
        if (!ALLOWED_HOSTS.contains(parsed.getHost())) {
            throw new SecurityException("Host not in allowlist");
        }
        // Resolve DNS and check for private IP ranges
        InetAddress resolved = InetAddress.getByName(parsed.getHost());
        if (resolved.isLoopbackAddress() || resolved.isSiteLocalAddress()
                || resolved.isLinkLocalAddress()) {
            throw new SecurityException("URL resolves to blocked IP range");
        }
        return parsed;
    }

    @Override
    protected void doGet(HttpServletRequest req, HttpServletResponse resp) throws Exception {
        String targetUrl = req.getParameter("url");
        try {
            URL validated = validateUrl(targetUrl);
            HttpURLConnection conn = (HttpURLConnection) validated.openConnection();
            conn.setInstanceFollowRedirects(false); // Prevent redirect-based SSRF
            InputStream in = conn.getInputStream();
            in.transferTo(resp.getOutputStream());
        } catch (SecurityException e) {
            resp.sendError(HttpServletResponse.SC_BAD_REQUEST, "Invalid URL");
        }
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `object_creation_expression`, `method_invocation`, `identifier`, `argument_list`
- **Detection approach**: Find `object_creation_expression` nodes constructing `new URL(...)` or `URI.create(...)` where the argument is an `identifier` tracing to user-controlled input (e.g., `req.getParameter()`, method parameters). Also detect `HttpRequest.newBuilder().uri()`, `RestTemplate.getForObject()`, and `OkHttpClient` calls with user-controlled URLs. Flag cases where no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(object_creation_expression
  type: (type_identifier) @type_name
  arguments: (argument_list
    (identifier) @url_arg))

(method_invocation
  object: (identifier) @obj
  name: (identifier) @method
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
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In Java web applications, this typically occurs through `HttpServletResponse.sendRedirect()` or Spring's `redirect:` view prefix with user-controlled arguments. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```java
import javax.servlet.http.*;

@WebServlet("/redirect")
public class RedirectServlet extends HttpServlet {
    @Override
    protected void doGet(HttpServletRequest req, HttpServletResponse resp) throws Exception {
        String target = req.getParameter("url");
        // User-controlled redirect target -- no validation
        resp.sendRedirect(target);
    }
}

// Spring MVC example
import org.springframework.stereotype.Controller;
import org.springframework.web.bind.annotation.*;

@Controller
public class LoginController {
    @GetMapping("/login-callback")
    public String loginCallback(@RequestParam(defaultValue = "/") String next) {
        // No validation on redirect destination
        return "redirect:" + next;
    }
}
```

### Good Code (Fix)
```java
import javax.servlet.http.*;
import java.net.URL;
import java.util.Set;

@WebServlet("/redirect")
public class RedirectServlet extends HttpServlet {
    private static final Set<String> ALLOWED_REDIRECT_DOMAINS =
        Set.of("example.com", "app.example.com");

    private boolean isSafeRedirect(String target) {
        // Allow relative paths (same-origin)
        if (target.startsWith("/") && !target.startsWith("//")) {
            return true;
        }
        try {
            URL parsed = new URL(target);
            return ALLOWED_REDIRECT_DOMAINS.contains(parsed.getHost());
        } catch (Exception e) {
            return false;
        }
    }

    @Override
    protected void doGet(HttpServletRequest req, HttpServletResponse resp) throws Exception {
        String target = req.getParameter("url");
        if (!isSafeRedirect(target)) {
            resp.sendError(HttpServletResponse.SC_BAD_REQUEST, "Invalid redirect target");
            return;
        }
        resp.sendRedirect(target);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_invocation`, `identifier`, `argument_list`, `string_literal`
- **Detection approach**: Find `method_invocation` nodes where the method is `sendRedirect` (on `HttpServletResponse`) and the argument is an `identifier` tracing to user-controlled input such as `req.getParameter()` or `@RequestParam`. Also detect Spring `redirect:` concatenation patterns where user input is appended to the redirect prefix. Flag when no URL validation or allowlist check precedes the redirect call.
- **S-expression query sketch**:
```scheme
(method_invocation
  object: (identifier) @resp_obj
  name: (identifier) @method
  arguments: (argument_list
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
