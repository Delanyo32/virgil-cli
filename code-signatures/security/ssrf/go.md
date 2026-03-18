# SSRF -- Go

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In Go applications, this commonly manifests through `http.Get()`, `http.Client.Do()`, or `http.NewRequest()` receiving unsanitized URLs from query parameters, form values, or JSON payloads. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: net/http (standard library), resty, fasthttp, gin, echo, fiber

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to an HTTP client function such as `http.Get()`, `http.Post()`, or constructing an `http.NewRequest()` with a user-controlled URL string without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```go
package main

import (
	"io"
	"net/http"
)

func proxyHandler(w http.ResponseWriter, r *http.Request) {
	targetURL := r.URL.Query().Get("url")
	// User-controlled URL passed directly to http.Get
	resp, err := http.Get(targetURL)
	if err != nil {
		http.Error(w, "Request failed", http.StatusInternalServerError)
		return
	}
	defer resp.Body.Close()
	io.Copy(w, resp.Body)
}

func fetchHandler(w http.ResponseWriter, r *http.Request) {
	targetURL := r.FormValue("url")
	// http.NewRequest with user-controlled URL
	req, _ := http.NewRequest("GET", targetURL, nil)
	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		http.Error(w, "Failed", http.StatusInternalServerError)
		return
	}
	defer resp.Body.Close()
	io.Copy(w, resp.Body)
}
```

### Good Code (Fix)
```go
package main

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
)

var allowedHosts = map[string]bool{
	"api.example.com": true,
	"cdn.example.com": true,
}

func validateURL(rawURL string) (*url.URL, error) {
	parsed, err := url.Parse(rawURL)
	if err != nil {
		return nil, fmt.Errorf("invalid URL: %w", err)
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return nil, fmt.Errorf("only HTTP(S) URLs are allowed")
	}
	if !allowedHosts[parsed.Hostname()] {
		return nil, fmt.Errorf("host not in allowlist")
	}
	// Resolve DNS and check for private IP ranges
	ips, err := net.LookupIP(parsed.Hostname())
	if err != nil {
		return nil, fmt.Errorf("cannot resolve hostname: %w", err)
	}
	for _, ip := range ips {
		if ip.IsLoopback() || ip.IsPrivate() || ip.IsLinkLocalUnicast() {
			return nil, fmt.Errorf("URL resolves to blocked IP range")
		}
	}
	return parsed, nil
}

func proxyHandler(w http.ResponseWriter, r *http.Request) {
	targetURL := r.URL.Query().Get("url")
	validated, err := validateURL(targetURL)
	if err != nil {
		http.Error(w, "Invalid URL", http.StatusBadRequest)
		return
	}
	resp, err := http.Get(validated.String())
	if err != nil {
		http.Error(w, "Request failed", http.StatusInternalServerError)
		return
	}
	defer resp.Body.Close()
	io.Copy(w, resp.Body)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes where the function is `http.Get`, `http.Post`, `http.NewRequest`, or `client.Do`, and the URL argument is an `identifier` that traces to user-controlled input (e.g., `r.URL.Query().Get()`, `r.FormValue()`). Flag cases where no URL validation occurs between the user input retrieval and the HTTP call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @package
    field: (field_identifier) @method)
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
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In Go web applications, this typically occurs through `http.Redirect()` with user-controlled arguments. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```go
package main

import "net/http"

func redirectHandler(w http.ResponseWriter, r *http.Request) {
	target := r.URL.Query().Get("url")
	// User-controlled redirect target -- no validation
	http.Redirect(w, r, target, http.StatusFound)
}

func loginCallback(w http.ResponseWriter, r *http.Request) {
	returnTo := r.FormValue("next")
	if returnTo == "" {
		returnTo = "/"
	}
	// No validation on redirect destination
	http.Redirect(w, r, returnTo, http.StatusSeeOther)
}
```

### Good Code (Fix)
```go
package main

import (
	"net/http"
	"net/url"
	"strings"
)

var allowedRedirectDomains = map[string]bool{
	"example.com":     true,
	"app.example.com": true,
}

func isSafeRedirect(target string) bool {
	// Allow relative paths (same-origin)
	if strings.HasPrefix(target, "/") && !strings.HasPrefix(target, "//") {
		return true
	}
	parsed, err := url.Parse(target)
	if err != nil {
		return false
	}
	return allowedRedirectDomains[parsed.Hostname()]
}

func redirectHandler(w http.ResponseWriter, r *http.Request) {
	target := r.URL.Query().Get("url")
	if !isSafeRedirect(target) {
		http.Error(w, "Invalid redirect target", http.StatusBadRequest)
		return
	}
	http.Redirect(w, r, target, http.StatusFound)
}

func loginCallback(w http.ResponseWriter, r *http.Request) {
	returnTo := r.FormValue("next")
	if returnTo == "" || !isSafeRedirect(returnTo) {
		returnTo = "/"
	}
	http.Redirect(w, r, returnTo, http.StatusSeeOther)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `selector_expression`, `identifier`, `argument_list`
- **Detection approach**: Find `call_expression` nodes where the function is `http.Redirect` and the URL argument (third positional argument) is an `identifier` that traces to user-controlled input such as `r.URL.Query().Get()` or `r.FormValue()`. Flag when no URL validation or allowlist check precedes the redirect call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (selector_expression
    operand: (identifier) @package
    field: (field_identifier) @method)
  arguments: (argument_list
    (_)
    (_)
    (identifier) @redirect_target
    (_)))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
