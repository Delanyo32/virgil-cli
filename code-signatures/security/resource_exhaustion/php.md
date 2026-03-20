# Resource Exhaustion -- PHP

## Overview
Resource exhaustion vulnerabilities in PHP arise from Regular Expression Denial of Service (ReDoS) via `preg_match()` with catastrophic backtracking patterns, and from reading entire file uploads or request streams into memory without size validation. PHP's PCRE regex engine uses backtracking and is susceptible to exponential time complexity on crafted input. The language's file handling functions readily consume unbounded memory when given large inputs.

## Why It's a Security Concern
ReDoS in PHP can exhaust the PCRE backtracking limit (causing silent failures or hanging the process depending on configuration) or lock up a PHP-FPM worker for the full `max_execution_time`. Since PHP-FPM has a finite worker pool, blocking multiple workers effectively takes down the entire application. Unbounded file reads can exhaust the memory limit per process or, with raised limits, exhaust server RAM entirely, crashing other co-hosted applications.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: PCRE (preg_match), Laravel, Symfony, file_get_contents, fread, php://input

---

## Pattern 1: ReDoS -- preg_match with Catastrophic Patterns

### Description
Using `preg_match()`, `preg_match_all()`, or `preg_replace()` with regular expression patterns containing nested quantifiers or overlapping alternations against user-supplied input. PHP's PCRE engine backtracks exponentially on these patterns. While `pcre.backtrack_limit` provides some protection, reaching it causes the match to return `false` (not an error), leading to logic bugs and potential bypasses.

### Bad Code (Anti-pattern)
```php
<?php

class InputValidator
{
    public function validateEmail(string $email): bool
    {
        // Nested quantifiers cause catastrophic backtracking
        $pattern = '/^([a-zA-Z0-9_\.\-]+)+@([a-zA-Z0-9\-]+\.)+[a-zA-Z]{2,}$/';
        return (bool) preg_match($pattern, $email);
    }

    public function extractTags(string $html): array
    {
        // Overlapping groups with quantifiers
        $pattern = '/(<[^>]*>)*.*(<\/[^>]*>)*/s';
        preg_match_all($pattern, $html, $matches);
        return $matches[0];
    }

    public function sanitize(string $input): string
    {
        // Nested repetition in replacement pattern
        return preg_replace('/(\s*\w+\s*=\s*"[^"]*"\s*)+/', '', $input);
    }
}
```

### Good Code (Fix)
```php
<?php

class InputValidator
{
    private const MAX_INPUT_LENGTH = 254;

    public function validateEmail(string $email): bool
    {
        if (strlen($email) > self::MAX_INPUT_LENGTH) {
            return false;
        }
        // Linear-time pattern without nested quantifiers
        $pattern = '/^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$/';
        return (bool) preg_match($pattern, $email);
    }

    public function extractTags(string $html): array
    {
        // Use a proper HTML parser
        $dom = new \DOMDocument();
        @$dom->loadHTML($html, LIBXML_NOERROR);
        $tags = [];
        foreach ($dom->getElementsByTagName('*') as $element) {
            $tags[] = $element->tagName;
        }
        return $tags;
    }

    public function sanitize(string $input): string
    {
        // Match individual attributes, not nested groups
        return preg_replace('/\w+="[^"]*"/', '', $input);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `argument`, `string`, `encapsed_string`, `name`
- **Detection approach**: Find `function_call_expression` nodes calling `preg_match`, `preg_match_all`, `preg_replace`, or `preg_replace_callback`. Extract the first argument (the pattern string). Analyze the regex between delimiters for nested quantifiers -- groups containing `+` or `*` that are themselves followed by `+` or `*`. Flag when the input argument (second parameter) is a variable rather than a constant.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (string) @pattern)
    (argument
      (_) @input)))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `regex_catastrophic_backtracking`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Reading Entire Upload into Memory Without Size Limit

### Description
Using `file_get_contents('php://input')`, `fread()` on upload streams, or `$_FILES` temporary file reads without first checking `Content-Length` or file size against a maximum. PHP's `upload_max_filesize` and `post_max_size` provide some protection, but custom stream handling or raised limits bypass these safeguards. Large payloads can exhaust the PHP process memory limit.

### Bad Code (Anti-pattern)
```php
<?php

class UploadHandler
{
    public function handleRawUpload(): string
    {
        // Reads entire request body with no size check
        $body = file_get_contents('php://input');
        return $this->processData($body);
    }

    public function handleFileUpload(): string
    {
        $tmpFile = $_FILES['document']['tmp_name'];
        // Reads entire file into memory without checking size
        $content = file_get_contents($tmpFile);
        return $this->processDocument($content);
    }

    public function streamUpload(): string
    {
        $handle = fopen('php://input', 'r');
        $data = '';
        // Accumulates without any size limit
        while (!feof($handle)) {
            $data .= fread($handle, 8192);
        }
        fclose($handle);
        return $data;
    }
}
```

### Good Code (Fix)
```php
<?php

class UploadHandler
{
    private const MAX_BODY_SIZE = 10 * 1024 * 1024; // 10 MB

    public function handleRawUpload(): string
    {
        $contentLength = (int) ($_SERVER['CONTENT_LENGTH'] ?? 0);
        if ($contentLength > self::MAX_BODY_SIZE) {
            throw new \RuntimeException('Payload too large');
        }
        $body = file_get_contents('php://input', false, null, 0, self::MAX_BODY_SIZE + 1);
        if (strlen($body) > self::MAX_BODY_SIZE) {
            throw new \RuntimeException('Payload too large');
        }
        return $this->processData($body);
    }

    public function handleFileUpload(): string
    {
        $tmpFile = $_FILES['document']['tmp_name'];
        $fileSize = filesize($tmpFile);
        if ($fileSize > self::MAX_BODY_SIZE) {
            throw new \RuntimeException('File too large');
        }
        $content = file_get_contents($tmpFile);
        return $this->processDocument($content);
    }

    public function streamUpload(): string
    {
        $handle = fopen('php://input', 'r');
        $data = '';
        $totalSize = 0;
        while (!feof($handle)) {
            $chunk = fread($handle, 8192);
            $totalSize += strlen($chunk);
            if ($totalSize > self::MAX_BODY_SIZE) {
                fclose($handle);
                throw new \RuntimeException('Payload too large');
            }
            $data .= $chunk;
        }
        fclose($handle);
        return $data;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `argument`, `string`, `name`, `member_access_expression`
- **Detection approach**: Find `function_call_expression` nodes calling `file_get_contents` where the first argument is `'php://input'` or a variable from `$_FILES`. Check the enclosing function for a preceding size check (`$_SERVER['CONTENT_LENGTH']` comparison or `filesize()` call). Also find `fread()` calls inside `while` loops on `php://input` without a total-size accumulator check. Flag when no size guard is present.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (argument
      (string) @source)))
```

### Pipeline Mapping
- **Pipeline name**: `resource_exhaustion`
- **Pattern name**: `unbounded_upload_read`
- **Severity**: warning
- **Confidence**: medium
