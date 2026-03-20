# Injection -- PHP

## Overview
Injection vulnerabilities are historically prevalent in PHP applications due to the language's tradition of directly embedding user input from superglobals (`$_GET`, `$_POST`, `$_REQUEST`) into SQL queries, shell commands, and `eval()` calls. While modern PHP offers PDO prepared statements and `escapeshellarg()`, legacy patterns and careless coding continue to produce critical injection vectors.

## Why It's a Security Concern
SQL injection through unparameterized queries can lead to full database compromise, authentication bypass, and data exfiltration. Command injection via `exec()`, `system()`, or `passthru()` grants attackers arbitrary OS command execution. Code injection through `eval()` allows running arbitrary PHP code with the web server's privileges. PHP applications are frequent targets for automated exploitation tools.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: PDO, MySQLi, mysql_* (deprecated), Laravel (raw queries), WordPress (wpdb), exec, system, passthru, shell_exec

---

## Pattern 1: SQL Injection via Variable Interpolation in Queries

### Description
Embedding PHP variables directly in SQL query strings passed to `mysql_query()`, `mysqli_query()`, `$pdo->query()`, or `$pdo->exec()`. PHP's double-quoted string interpolation (`"...$var..."`) and concatenation make this pattern especially common.

### Bad Code (Anti-pattern)
```php
function getUser($pdo, $userId) {
    $query = "SELECT * FROM users WHERE id = '$userId'";
    $stmt = $pdo->query($query);
    return $stmt->fetch(PDO::FETCH_ASSOC);
}

function searchProducts($conn, $name) {
    $result = mysqli_query($conn, "SELECT * FROM products WHERE name LIKE '%" . $name . "%'");
    return mysqli_fetch_all($result, MYSQLI_ASSOC);
}
```

### Good Code (Fix)
```php
function getUser($pdo, $userId) {
    $stmt = $pdo->prepare("SELECT * FROM users WHERE id = :userId");
    $stmt->execute([':userId' => $userId]);
    return $stmt->fetch(PDO::FETCH_ASSOC);
}

function searchProducts($pdo, $name) {
    $stmt = $pdo->prepare("SELECT * FROM products WHERE name LIKE :name");
    $stmt->execute([':name' => "%{$name}%"]);
    return $stmt->fetchAll(PDO::FETCH_ASSOC);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `member_call_expression`, `encapsed_string`, `binary_expression`, `variable_name`
- **Detection approach**: Find `function_call_expression` or `member_call_expression` nodes calling `query`, `exec`, `mysql_query`, or `mysqli_query` where the argument is an `encapsed_string` (double-quoted string with `$variable` interpolation) or a `binary_expression` (concatenation) containing `variable_name` nodes. Confirm SQL keywords in the string portions.
- **S-expression query sketch**:
```scheme
(member_call_expression
  name: (name) @method
  arguments: (arguments
    (encapsed_string
      (variable_name) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_variable_interpolation`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via exec/system/passthru

### Description
Passing user-controlled input to `exec()`, `system()`, `passthru()`, `shell_exec()`, or the backtick operator without proper escaping via `escapeshellarg()` or `escapeshellcmd()`. Attackers can inject shell metacharacters to execute arbitrary commands.

### Bad Code (Anti-pattern)
```php
function convertImage($filename) {
    exec("convert " . $filename . " output.png", $output, $returnCode);
    return $returnCode === 0;
}

function pingHost($host) {
    $result = shell_exec("ping -c 4 $host");
    return $result;
}
```

### Good Code (Fix)
```php
function convertImage($filename) {
    $safeFilename = escapeshellarg($filename);
    exec("convert " . $safeFilename . " output.png", $output, $returnCode);
    return $returnCode === 0;
}

function pingHost($host) {
    // Validate input format
    if (!filter_var($host, FILTER_VALIDATE_IP) && !filter_var($host, FILTER_VALIDATE_DOMAIN)) {
        throw new InvalidArgumentException("Invalid host");
    }
    $safeHost = escapeshellarg($host);
    $result = shell_exec("ping -c 4 " . $safeHost);
    return $result;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `encapsed_string`, `binary_expression`, `variable_name`
- **Detection approach**: Find `function_call_expression` nodes calling `exec`, `system`, `passthru`, `shell_exec`, or `popen` where the argument is an `encapsed_string` containing `variable_name` nodes or a `binary_expression` concatenating a variable without a prior call to `escapeshellarg()`. Also detect backtick expressions containing variables.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (encapsed_string
      (variable_name) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_exec`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: Code Injection via eval with User Input

### Description
Passing user-controlled strings to `eval()`, which executes them as PHP code. This is one of the most dangerous PHP functions when used with external input, granting attackers full code execution capabilities.

### Bad Code (Anti-pattern)
```php
function calculate($expression) {
    return eval("return $expression;");
}

function runTemplate($code) {
    $template = $_POST['template'];
    eval("?>" . $template);
}
```

### Good Code (Fix)
```php
function calculate($expression) {
    // Use a math expression parser instead of eval
    $parser = new MathParser();
    return $parser->evaluate($expression);
}

function runTemplate($templateName) {
    // Use a proper template engine
    $allowedTemplates = ['header', 'footer', 'sidebar'];
    if (!in_array($templateName, $allowedTemplates)) {
        throw new InvalidArgumentException("Unknown template");
    }
    include __DIR__ . "/templates/{$templateName}.php";
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression`, `name`, `encapsed_string`, `binary_expression`, `variable_name`
- **Detection approach**: Find `function_call_expression` nodes where the function name is `eval` and the argument contains a `variable_name`, `encapsed_string` with interpolation, or a `binary_expression` incorporating non-literal values. Pure string literal arguments are lower risk but still warrant review.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (encapsed_string
      (variable_name) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `code_injection_eval`
- **Severity**: error
- **Confidence**: high
