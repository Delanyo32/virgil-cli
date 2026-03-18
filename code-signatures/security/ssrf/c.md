# SSRF -- C

## Overview
Server-Side Request Forgery (SSRF) occurs when a server-side application makes HTTP requests to URLs that are fully or partially controlled by user input. In C applications, this commonly manifests through libcurl's `curl_easy_setopt()` with `CURLOPT_URL` receiving unsanitized URLs from command-line arguments, configuration files, network input, or CGI/FastCGI parameters. Attackers exploit SSRF to access internal services, cloud metadata endpoints (e.g., `169.254.169.254`), or to bypass network-level access controls.

## Why It's a Security Concern
SSRF enables attackers to pivot from an external-facing application into internal infrastructure. Common exploitation targets include cloud provider metadata services (AWS IMDSv1, GCP metadata), internal APIs, databases, and admin panels that are not exposed to the public internet. In C applications, SSRF is particularly dangerous because libcurl supports many protocols beyond HTTP (FTP, TFTP, LDAP, Gopher, file://), and C programs often run with elevated privileges or as system daemons. SSRF is ranked in the OWASP Top 10 (A10:2021).

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: libcurl, libsoup, libevent (evhttp), microhttpd, CGI/FastCGI programs

---

## Pattern 1: User-Controlled URL in HTTP Request

### Description
Passing user-supplied input directly as the URL argument to `curl_easy_setopt(curl, CURLOPT_URL, user_url)` without validating the URL scheme, hostname, or resolved IP address against an allowlist. An attacker can supply URLs targeting internal services, cloud metadata endpoints, or leverage libcurl's multi-protocol support for file:// or gopher:// access.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>
#include <curl/curl.h>

/* CGI program that proxies user-specified URL */
int main(void) {
    char *query_string = getenv("QUERY_STRING");
    char user_url[1024];
    /* Extract url= parameter from query string */
    sscanf(query_string, "url=%1023s", user_url);

    CURL *curl = curl_easy_init();
    if (curl) {
        /* User-controlled URL passed directly to libcurl */
        curl_easy_setopt(curl, CURLOPT_URL, user_url);
        curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, NULL);
        curl_easy_perform(curl);
        curl_easy_cleanup(curl);
    }
    return 0;
}

/* Function taking URL from network input */
void fetch_resource(const char *url_from_client) {
    CURL *curl = curl_easy_init();
    /* No validation on the URL */
    curl_easy_setopt(curl, CURLOPT_URL, url_from_client);
    curl_easy_perform(curl);
    curl_easy_cleanup(curl);
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <string.h>
#include <curl/curl.h>
#include <netdb.h>
#include <arpa/inet.h>

static const char *allowed_hosts[] = {"api.example.com", "cdn.example.com", NULL};

static int is_private_ip(const char *ip) {
    struct in_addr addr;
    if (inet_pton(AF_INET, ip, &addr) != 1) return 1;
    unsigned long host = ntohl(addr.s_addr);
    /* 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8, 169.254.0.0/16 */
    if ((host >> 24) == 10) return 1;
    if ((host >> 20) == (172 * 16 + 1)) return 1;  /* 172.16-31.x.x */
    if ((host >> 16) == (192 * 256 + 168)) return 1;
    if ((host >> 24) == 127) return 1;
    if ((host >> 16) == (169 * 256 + 254)) return 1;
    return 0;
}

static int validate_url(const char *url, char *validated, size_t len) {
    CURLU *cu = curl_url();
    if (curl_url_set(cu, CURLUPART_URL, url, 0) != CURLUE_OK) {
        curl_url_cleanup(cu);
        return -1;
    }
    char *scheme = NULL, *host = NULL;
    curl_url_get(cu, CURLUPART_SCHEME, &scheme, 0);
    curl_url_get(cu, CURLUPART_HOST, &host, 0);

    /* Only allow HTTP(S) */
    if (strcmp(scheme, "http") != 0 && strcmp(scheme, "https") != 0) {
        curl_free(scheme); curl_free(host); curl_url_cleanup(cu);
        return -1;
    }
    /* Check host allowlist */
    int found = 0;
    for (int i = 0; allowed_hosts[i]; i++) {
        if (strcmp(host, allowed_hosts[i]) == 0) { found = 1; break; }
    }
    if (!found) {
        curl_free(scheme); curl_free(host); curl_url_cleanup(cu);
        return -1;
    }
    /* Resolve DNS and check for private IPs */
    struct hostent *he = gethostbyname(host);
    if (he) {
        char ip[INET_ADDRSTRLEN];
        inet_ntop(AF_INET, he->h_addr_list[0], ip, sizeof(ip));
        if (is_private_ip(ip)) {
            curl_free(scheme); curl_free(host); curl_url_cleanup(cu);
            return -1;
        }
    }
    snprintf(validated, len, "%s", url);
    curl_free(scheme); curl_free(host); curl_url_cleanup(cu);
    return 0;
}

int main(void) {
    char *query_string = getenv("QUERY_STRING");
    char user_url[1024], validated_url[1024];
    sscanf(query_string, "url=%1023s", user_url);

    if (validate_url(user_url, validated_url, sizeof(validated_url)) != 0) {
        printf("Status: 400\r\n\r\nInvalid URL\n");
        return 1;
    }
    CURL *curl = curl_easy_init();
    if (curl) {
        curl_easy_setopt(curl, CURLOPT_URL, validated_url);
        curl_easy_setopt(curl, CURLOPT_PROTOCOLS_STR, "http,https"); /* Block non-HTTP protocols */
        curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 0L);         /* Prevent redirect-based SSRF */
        curl_easy_perform(curl);
        curl_easy_cleanup(curl);
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `string_literal`
- **Detection approach**: Find `call_expression` nodes where the function is `curl_easy_setopt` and the second argument is the identifier `CURLOPT_URL`, then check if the third argument is an `identifier` (variable) rather than a `string_literal`. Flag cases where the URL variable originates from user-controlled input (e.g., `getenv()`, `argv[]`, network read functions) and no URL validation occurs before the curl call.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (identifier) @curl_handle
    (identifier) @option
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
Setting HTTP redirect headers with user-supplied URLs without verifying that the target is a same-origin relative path or belongs to an allowlisted set of domains. In C web applications (CGI, FastCGI, embedded HTTP servers), this typically occurs when user input is directly interpolated into a `Location:` header. Open redirects become SSRF vectors when server-side redirect-following HTTP clients chase them, or serve as phishing attack components.

### Bad Code (Anti-pattern)
```c
#include <stdio.h>
#include <stdlib.h>

/* CGI program performing redirect based on user input */
int main(void) {
    char *query_string = getenv("QUERY_STRING");
    char target[1024];
    sscanf(query_string, "url=%1023s", target);

    /* User-controlled redirect target -- no validation */
    printf("Status: 302\r\n");
    printf("Location: %s\r\n", target);
    printf("\r\n");
    return 0;
}

/* Embedded HTTP server handler */
void handle_redirect(const char *user_target, int client_fd) {
    char response[2048];
    /* No validation on redirect destination */
    snprintf(response, sizeof(response),
        "HTTP/1.1 302 Found\r\nLocation: %s\r\n\r\n", user_target);
    write(client_fd, response, strlen(response));
}
```

### Good Code (Fix)
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *allowed_redirect_domains[] = {"example.com", "app.example.com", NULL};

static int is_safe_redirect(const char *target) {
    /* Allow relative paths (same-origin) */
    if (target[0] == '/' && (strlen(target) < 2 || target[1] != '/')) {
        return 1;
    }
    /* Parse and check against allowlist */
    CURLU *cu = curl_url();
    if (curl_url_set(cu, CURLUPART_URL, target, 0) != CURLUE_OK) {
        curl_url_cleanup(cu);
        return 0;
    }
    char *host = NULL;
    curl_url_get(cu, CURLUPART_HOST, &host, 0);
    int safe = 0;
    if (host) {
        for (int i = 0; allowed_redirect_domains[i]; i++) {
            if (strcmp(host, allowed_redirect_domains[i]) == 0) {
                safe = 1;
                break;
            }
        }
        curl_free(host);
    }
    curl_url_cleanup(cu);
    return safe;
}

int main(void) {
    char *query_string = getenv("QUERY_STRING");
    char target[1024];
    sscanf(query_string, "url=%1023s", target);

    if (!is_safe_redirect(target)) {
        printf("Status: 400\r\n\r\nInvalid redirect target\n");
        return 1;
    }
    printf("Status: 302\r\n");
    printf("Location: %s\r\n", target);
    printf("\r\n");
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `identifier`, `argument_list`, `string_literal`
- **Detection approach**: Find `call_expression` nodes where the function is `printf`, `snprintf`, `fprintf`, or `write`, and the format string or buffer contains `"Location:"` with a format specifier (`%s`) followed by an `identifier` argument that traces to user-controlled input. Flag when no URL validation or allowlist check precedes the redirect header output.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (argument_list
    (string_literal) @format_string
    (identifier) @redirect_target))
```

### Pipeline Mapping
- **Pipeline name**: `ssrf`
- **Pattern name**: `open_redirect_user_input`
- **Severity**: error
- **Confidence**: high
