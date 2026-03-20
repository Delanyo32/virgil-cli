# SSRF -- Rust

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In Rust applications, this commonly manifests through `reqwest`, `hyper`, or `ureq` HTTP clients receiving unsanitized URLs from request parameters or API payloads. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: reqwest, hyper, ureq, actix-web, axum, warp, rocket

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to an HTTP client function such as `reqwest::get()`, `reqwest::Client::get()`, or `ureq::get()` without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```rust
use actix_web::{web, HttpResponse};
use reqwest;

async fn proxy(query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    let user_url = query.get("url").unwrap();
    // User-controlled URL passed directly to reqwest
    let response = reqwest::get(user_url).await.unwrap();
    let body = response.text().await.unwrap();
    HttpResponse::Ok().body(body)
}

async fn fetch_resource(target_url: &str) -> Result<String, reqwest::Error> {
    // No validation on the URL before making the request
    let client = reqwest::Client::new();
    let resp = client.get(target_url).send().await?;
    resp.text().await
}
```

### Good Code (Fix)
```rust
use actix_web::{web, HttpResponse};
use reqwest;
use url::Url;
use std::net::{IpAddr, ToSocketAddrs};
use std::collections::HashSet;

fn validate_url(input: &str) -> Result<Url, String> {
    let parsed = Url::parse(input).map_err(|e| format!("Invalid URL: {}", e))?;

    // Only allow HTTP(S)
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err("Only HTTP(S) URLs are allowed".into()),
    }

    let allowed_hosts: HashSet<&str> = ["api.example.com", "cdn.example.com"]
        .iter().copied().collect();

    let host = parsed.host_str().ok_or("No host in URL")?;
    if !allowed_hosts.contains(host) {
        return Err("Host not in allowlist".into());
    }

    // Resolve DNS and check for private IP ranges
    let addr_str = format!("{}:{}", host, parsed.port_or_known_default().unwrap_or(443));
    if let Ok(addrs) = addr_str.to_socket_addrs() {
        for addr in addrs {
            match addr.ip() {
                IpAddr::V4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => {
                    return Err("URL resolves to blocked IP range".into());
                }
                _ => {}
            }
        }
    }

    Ok(parsed)
}

async fn proxy(query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    let user_url = match query.get("url") {
        Some(u) => u,
        None => return HttpResponse::BadRequest().body("Missing url parameter"),
    };
    let validated = match validate_url(user_url) {
        Ok(u) => u,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    match reqwest::get(validated.as_str()).await {
        Ok(resp) => HttpResponse::Ok().body(resp.text().await.unwrap_or_default()),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `field_expression`, `identifier`
- **Detection approach**: Find `call_expression` nodes where the function is `reqwest::get`, `client.get`, `ureq::get`, or `hyper::Client::get`, and the first argument is an `identifier` or reference expression that traces to user-controlled input (e.g., query parameters, path segments from web framework extractors). Flag cases where no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @module
    name: (identifier) @method)
  arguments: (arguments
    (reference_expression
      (identifier) @url_arg)))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `user_controlled_url_http_request`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Open Redirect via User-Controlled Redirect Target

### Description
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In Rust web frameworks (actix-web, axum, warp, rocket), this typically occurs through redirect response constructors with user-controlled location headers. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```rust
use actix_web::{web, HttpResponse};

async fn handle_redirect(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let target = query.get("url").unwrap();
    // User-controlled redirect target -- no validation
    HttpResponse::Found()
        .append_header(("Location", target.as_str()))
        .finish()
}

// Axum example
use axum::{extract::Query, response::Redirect};

async fn login_callback(Query(params): Query<std::collections::HashMap<String, String>>) -> Redirect {
    let return_to = params.get("next").cloned().unwrap_or_else(|| "/".to_string());
    // No validation on redirect destination
    Redirect::to(&return_to)
}
```

### Good Code (Fix)
```rust
use actix_web::{web, HttpResponse};
use url::Url;
use std::collections::HashSet;

fn is_safe_redirect(target: &str) -> bool {
    // Allow relative paths (same-origin)
    if target.starts_with('/') && !target.starts_with("//") {
        return true;
    }
    let allowed_domains: HashSet<&str> = ["example.com", "app.example.com"]
        .iter().copied().collect();
    if let Ok(parsed) = Url::parse(target) {
        if let Some(host) = parsed.host_str() {
            return allowed_domains.contains(host);
        }
    }
    false
}

async fn handle_redirect(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let target = match query.get("url") {
        Some(t) if is_safe_redirect(t) => t.clone(),
        _ => return HttpResponse::BadRequest().body("Invalid redirect target"),
    };
    HttpResponse::Found()
        .append_header(("Location", target.as_str()))
        .finish()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `string_literal`, `identifier`
- **Detection approach**: Find `call_expression` nodes constructing redirect responses (`HttpResponse::Found()`, `Redirect::to()`, `Redirect::temporary()`) where the `Location` header value or redirect argument is an `identifier` or expression tracing to user-controlled input. Flag when no URL validation or allowlist check precedes the redirect construction.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @type_name
    name: (identifier) @method)
  arguments: (arguments
    (reference_expression
      (identifier) @redirect_target)))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
