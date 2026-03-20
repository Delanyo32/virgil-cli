# SSRF -- Python

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In Python applications, this commonly manifests through `requests.get()`, `urllib.request.urlopen()`, `httpx`, or `aiohttp` receiving unsanitized URLs from request parameters, form data, or API payloads. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. SSRF can lead to credential theft, remote code execution on internal hosts, data exfiltration, and full cloud account takeover. It is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Frameworks/libraries**: requests, urllib, urllib3, httpx, aiohttp, flask, django, fastapi

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to an HTTP client function such as `requests.get()`, `urllib.request.urlopen()`, or `httpx.get()` without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or arbitrary external hosts.

### Bad Code (Anti-pattern)
```python
from flask import Flask, request
import requests

app = Flask(__name__)

@app.route('/proxy')
def proxy():
    target_url = request.args.get('url')
    # User-controlled URL passed directly to requests
    response = requests.get(target_url)
    return response.text

@app.route('/fetch')
def fetch_page():
    url = request.args.get('url')
    # urllib with user-controlled URL
    import urllib.request
    response = urllib.request.urlopen(url)
    return response.read()
```

### Good Code (Fix)
```python
from flask import Flask, request, abort
import requests
import ipaddress
import socket
from urllib.parse import urlparse

app = Flask(__name__)

ALLOWED_HOSTS = {'api.example.com', 'cdn.example.com'}
ALLOWED_SCHEMES = {'http', 'https'}

def validate_url(url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme not in ALLOWED_SCHEMES:
        raise ValueError('Only HTTP(S) URLs are allowed')
    if parsed.hostname not in ALLOWED_HOSTS:
        raise ValueError('Host not in allowlist')
    # Resolve DNS and check for private IP ranges
    try:
        resolved_ip = socket.gethostbyname(parsed.hostname)
        ip = ipaddress.ip_address(resolved_ip)
        if ip.is_private or ip.is_loopback or ip.is_link_local:
            raise ValueError('URL resolves to blocked IP range')
    except socket.gaierror:
        raise ValueError('Cannot resolve hostname')
    return url

@app.route('/proxy')
def proxy():
    target_url = request.args.get('url')
    try:
        validated = validate_url(target_url)
    except ValueError as e:
        abort(400, str(e))
    response = requests.get(validated)
    return response.text
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `attribute`, `argument_list`, `identifier`
- **Detection approach**: Find `call` nodes where the function is `requests.get`, `requests.post`, `requests.request`, `urllib.request.urlopen`, `httpx.get`, or `aiohttp.ClientSession().get`, and the first positional argument is an `identifier` that traces to user-controlled input (e.g., `request.args.get()`, `request.form`, function parameters). Flag cases where no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(call
  function: (attribute
    object: (identifier) @module
    attribute: (identifier) @method)
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
Redirecting the client to a URL supplied by user input without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In Python web frameworks (Flask, Django, FastAPI), this typically occurs through `redirect()` calls with user-controlled arguments. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```python
from flask import Flask, request, redirect

app = Flask(__name__)

@app.route('/redirect')
def handle_redirect():
    target = request.args.get('url')
    # User-controlled redirect target
    return redirect(target)

@app.route('/login-callback')
def login_callback():
    return_to = request.args.get('next', '/')
    # No validation on redirect destination
    return redirect(return_to)
```

### Good Code (Fix)
```python
from flask import Flask, request, redirect, abort
from urllib.parse import urlparse

app = Flask(__name__)

ALLOWED_REDIRECT_DOMAINS = {'example.com', 'app.example.com'}

def is_safe_redirect(target: str) -> bool:
    # Allow relative paths (same-origin)
    if target.startswith('/') and not target.startswith('//'):
        return True
    try:
        parsed = urlparse(target)
        return parsed.hostname in ALLOWED_REDIRECT_DOMAINS
    except Exception:
        return False

@app.route('/redirect')
def handle_redirect():
    target = request.args.get('url')
    if not is_safe_redirect(target):
        abort(400, 'Invalid redirect target')
    return redirect(target)

@app.route('/login-callback')
def login_callback():
    return_to = request.args.get('next', '/')
    if not is_safe_redirect(return_to):
        return redirect('/')
    return redirect(return_to)
```

### Tree-sitter Detection Strategy
- **Target node types**: `call`, `identifier`, `argument_list`, `keyword_argument`
- **Detection approach**: Find `call` nodes where the function is `redirect` (Flask/Django) or `RedirectResponse` (FastAPI/Starlette) and the first argument is an `identifier` or expression that traces to user-controlled input such as `request.args.get()`, `request.GET.get()`, or function parameters. Flag when no URL validation or allowlist check precedes the redirect call.
- **S-expression query sketch**:
```scheme
(call
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
