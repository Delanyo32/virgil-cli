# SSRF -- C++

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In C++ applications, this commonly manifests through libcurl's `curl_easy_setopt()` with `CURLOPT_URL`, cpp-httplib, Boost.Beast, or cpr library receiving unsanitized URLs from user input, network data, or configuration sources. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. In C++ applications, SSRF is particularly dangerous because libcurl supports many protocols beyond HTTP (FTP, TFTP, LDAP, Gopher, file://), and C++ services often run as long-lived server processes with access to internal networks. SSRF is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Frameworks/libraries**: libcurl (C++ wrapper), cpr, cpp-httplib, Boost.Beast, Poco::Net, Qt Network (QNetworkAccessManager)

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to `curl_easy_setopt(curl, CURLOPT_URL, userUrl.c_str())`, `cpr::Get(cpr::Url{userUrl})`, `httplib::Client`, or similar HTTP client methods without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or leverage multi-protocol support for file:// or gopher:// access.

### Bad Code (Anti-pattern)
```cpp
#include <iostream>
#include <string>
#include <curl/curl.h>

// libcurl with user-controlled URL
void fetchUrl(const std::string& userUrl) {
    CURL* curl = curl_easy_init();
    if (curl) {
        // User-controlled URL passed directly to libcurl
        curl_easy_setopt(curl, CURLOPT_URL, userUrl.c_str());
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, nullptr);
        curl_easy_perform(curl);
        curl_easy_cleanup(curl);
    }
}

// cpr library with user-controlled URL
#include <cpr/cpr.h>

std::string proxyRequest(const std::string& targetUrl) {
    // No validation on the URL
    auto response = cpr::Get(cpr::Url{targetUrl});
    return response.text;
}

// Web handler
void handleProxy(const HttpRequest& req, HttpResponse& resp) {
    std::string url = req.getParam("url");
    resp.setBody(proxyRequest(url));
}
```

### Good Code (Fix)
```cpp
#include <iostream>
#include <string>
#include <set>
#include <curl/curl.h>
#include <netdb.h>
#include <arpa/inet.h>

static const std::set<std::string> ALLOWED_HOSTS = {"api.example.com", "cdn.example.com"};

bool isPrivateIp(const std::string& ip) {
    struct in_addr addr;
    if (inet_pton(AF_INET, ip.c_str(), &addr) != 1) return true;
    uint32_t host = ntohl(addr.s_addr);
    if ((host >> 24) == 10) return true;       // 10.0.0.0/8
    if ((host >> 20) == 0xAC1) return true;    // 172.16.0.0/12
    if ((host >> 16) == 0xC0A8) return true;   // 192.168.0.0/16
    if ((host >> 24) == 127) return true;      // 127.0.0.0/8
    if ((host >> 16) == 0xA9FE) return true;   // 169.254.0.0/16
    return false;
}

std::string validateUrl(const std::string& input) {
    CURLU* cu = curl_url();
    if (curl_url_set(cu, CURLUPART_URL, input.c_str(), 0) != CURLUE_OK) {
        curl_url_cleanup(cu);
        throw std::invalid_argument("Invalid URL");
    }
    char* scheme = nullptr;
    char* host = nullptr;
    curl_url_get(cu, CURLUPART_SCHEME, &scheme, 0);
    curl_url_get(cu, CURLUPART_HOST, &host, 0);

    std::string schemeStr(scheme ? scheme : "");
    std::string hostStr(host ? host : "");
    curl_free(scheme);
    curl_free(host);
    curl_url_cleanup(cu);

    if (schemeStr != "http" && schemeStr != "https") {
        throw std::invalid_argument("Only HTTP(S) URLs are allowed");
    }
    if (ALLOWED_HOSTS.find(hostStr) == ALLOWED_HOSTS.end()) {
        throw std::invalid_argument("Host not in allowlist");
    }
    // Resolve DNS and check for private IP ranges
    struct hostent* he = gethostbyname(hostStr.c_str());
    if (he) {
        char ip[INET_ADDRSTRLEN];
        inet_ntop(AF_INET, he->h_addr_list[0], ip, sizeof(ip));
        if (isPrivateIp(ip)) {
            throw std::invalid_argument("URL resolves to blocked IP range");
        }
    }
    return input;
}

void fetchUrl(const std::string& userUrl) {
    std::string validated = validateUrl(userUrl);
    CURL* curl = curl_easy_init();
    if (curl) {
        curl_easy_setopt(curl, CURLOPT_URL, validated.c_str());
        curl_easy_setopt(curl, CURLOPT_PROTOCOLS_STR, "http,https");
        curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 0L);
        curl_easy_perform(curl);
        curl_easy_cleanup(curl);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `string_literal`, `field_expression`
- **Detection approach**: Find `call_expression` nodes where the function is `curl_easy_setopt` and the second argument is `CURLOPT_URL`, then check if the third argument involves a method call on a variable (e.g., `.c_str()`) or is a direct `identifier`. Also detect `cpr::Get(cpr::Url{...})` or `httplib::Client` constructor calls with user-controlled URLs. Flag cases where the URL variable originates from user input and no URL validation occurs before the HTTP call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @curl_handle
    (identifier) @option
    (call_expression
      function: (field_expression
        argument: (identifier) @url_var
        field: (field_identifier) @method))))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `user_controlled_url_http_request`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Open Redirect via User-Controlled Redirect Target

### Description
Setting HTTP redirect headers with user-supplied URLs without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In C++ web applications (embedded HTTP servers, CGI, web frameworks like Crow, Drogon, or Pistache), this typically occurs when user input is directly used in a `Location` header or redirect response. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```cpp
#include <string>

// Crow framework example
#include "crow.h"

int main() {
    crow::SimpleApp app;

    CROW_ROUTE(app, "/redirect")([](const crow::request& req) {
        std::string target = req.url_params.get("url");
        // User-controlled redirect target -- no validation
        crow::response resp(302);
        resp.set_header("Location", target);
        return resp;
    });

    app.port(8080).run();
}

// Raw HTTP response with user-controlled redirect
void handleRedirect(const std::string& userTarget, int clientFd) {
    std::string response = "HTTP/1.1 302 Found\r\nLocation: " + userTarget + "\r\n\r\n";
    // No validation on redirect destination
    write(clientFd, response.c_str(), response.size());
}
```

### Good Code (Fix)
```cpp
#include <string>
#include <set>
#include <curl/curl.h>
#include "crow.h"

static const std::set<std::string> ALLOWED_REDIRECT_DOMAINS = {
    "example.com", "app.example.com"
};

bool isSafeRedirect(const std::string& target) {
    // Allow relative paths (same-origin)
    if (!target.empty() && target[0] == '/' &&
        (target.size() < 2 || target[1] != '/')) {
        return true;
    }
    CURLU* cu = curl_url();
    if (curl_url_set(cu, CURLUPART_URL, target.c_str(), 0) != CURLUE_OK) {
        curl_url_cleanup(cu);
        return false;
    }
    char* host = nullptr;
    curl_url_get(cu, CURLUPART_HOST, &host, 0);
    bool safe = false;
    if (host) {
        safe = ALLOWED_REDIRECT_DOMAINS.count(std::string(host)) > 0;
        curl_free(host);
    }
    curl_url_cleanup(cu);
    return safe;
}

int main() {
    crow::SimpleApp app;

    CROW_ROUTE(app, "/redirect")([](const crow::request& req) {
        std::string target = req.url_params.get("url");
        if (!isSafeRedirect(target)) {
            return crow::response(400, "Invalid redirect target");
        }
        crow::response resp(302);
        resp.set_header("Location", target);
        return resp;
    });

    app.port(8080).run();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `identifier`, `argument_list`, `string_literal`
- **Detection approach**: Find `call_expression` nodes where the method is `set_header` with a `"Location"` string literal and the value argument is an `identifier` or expression tracing to user-controlled input. Also detect string concatenation patterns building HTTP response strings containing `"Location:"` with a variable. Flag when no URL validation or allowlist check precedes the redirect header construction.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    argument: (identifier) @resp_obj
    field: (field_identifier) @method)
  arguments: (argument_list
    (string_literal) @header_name
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
