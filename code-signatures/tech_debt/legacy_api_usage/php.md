# Legacy API Usage -- PHP

## Overview
Legacy API usage in PHP refers to relying on deprecated or dangerous functions and patterns when modern, safer alternatives exist. Common examples include using the removed `mysql_*` extension instead of PDO or MySQLi, using `extract()` to spread array keys into local variables, and placing business logic directly in view/template files.

## Why It's a Tech Debt Concern
The `mysql_*` functions were removed in PHP 7.0 and lack prepared statement support, making code vulnerable to SQL injection. `extract()` creates variables with unpredictable names from untrusted data, making code difficult to trace and opening security holes. Business logic in views violates separation of concerns, making templates untestable and forcing duplication when the same logic is needed in an API response or CLI command. All three patterns signal a codebase that has not been modernized and increase the cost of every subsequent change.

## Applicability
- **Relevance**: high (these patterns are extremely common in legacy PHP codebases, especially those predating PHP 7 and modern frameworks)
- **Languages covered**: `.php`
- **Frameworks/libraries**: WordPress (common source of all three patterns), custom MVC frameworks, legacy CMS systems

---

## Pattern 1: Deprecated mysql_* Functions

### Description
Using the removed `mysql_*` extension functions (`mysql_connect`, `mysql_query`, `mysql_fetch_array`, `mysql_real_escape_string`, etc.) instead of PDO or MySQLi. The `mysql_*` extension was deprecated in PHP 5.5 and removed entirely in PHP 7.0. It does not support prepared statements, making all queries vulnerable to SQL injection when user input is interpolated.

### Bad Code (Anti-pattern)
```php
<?php
$conn = mysql_connect('localhost', 'root', 'password');
mysql_select_db('myapp', $conn);

function getUser($id) {
    global $conn;
    $id = mysql_real_escape_string($id, $conn);
    $result = mysql_query("SELECT * FROM users WHERE id = '$id'", $conn);

    if (!$result) {
        die('Query failed: ' . mysql_error($conn));
    }

    return mysql_fetch_assoc($result);
}

function searchUsers($name) {
    global $conn;
    // SQL injection vulnerability: escape is insufficient for LIKE
    $name = mysql_real_escape_string($name, $conn);
    $result = mysql_query("SELECT * FROM users WHERE name LIKE '%$name%'", $conn);

    $users = [];
    while ($row = mysql_fetch_array($result)) {
        $users[] = $row;
    }
    mysql_free_result($result);
    return $users;
}
```

### Good Code (Fix)
```php
<?php
$pdo = new PDO(
    'mysql:host=localhost;dbname=myapp;charset=utf8mb4',
    'root',
    'password',
    [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
        PDO::ATTR_EMULATE_PREPARES => false,
    ]
);

function getUser(PDO $pdo, int $id): ?array {
    $stmt = $pdo->prepare('SELECT * FROM users WHERE id = :id');
    $stmt->execute(['id' => $id]);
    return $stmt->fetch() ?: null;
}

function searchUsers(PDO $pdo, string $name): array {
    $stmt = $pdo->prepare('SELECT * FROM users WHERE name LIKE :name');
    $stmt->execute(['name' => "%{$name}%"]);
    return $stmt->fetchAll();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression` with `name` matching `mysql_*`
- **Detection approach**: Find `function_call_expression` nodes whose `name` child matches the pattern `mysql_*` (e.g., `mysql_connect`, `mysql_query`, `mysql_fetch_assoc`, `mysql_real_escape_string`, `mysql_close`, `mysql_select_db`, `mysql_error`, `mysql_num_rows`, `mysql_free_result`, `mysql_fetch_array`). Every occurrence is a direct replacement candidate.
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  (#match? @func_name "^mysql_"))
```

### Pipeline Mapping
- **Pipeline name**: `deprecated_mysql_api`
- **Pattern name**: `mysql_extension_usage`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: extract() Usage

### Description
Using `extract()` to spread an associative array's key-value pairs into local variables. This makes it impossible to determine which variables exist in the current scope without tracing the array's contents at runtime. When used with user input (`$_GET`, `$_POST`, `$_REQUEST`), it allows attackers to overwrite arbitrary local variables.

### Bad Code (Anti-pattern)
```php
<?php
function processForm(array $data): void {
    extract($data);
    // What variables exist here? Impossible to know without inspecting $data
    // If $data = ['name' => 'Alice', 'role' => 'admin'], now $name and $role exist
    echo "Hello, $name! Your role is $role.";
}

// Dangerous: extracting user input
extract($_POST);
// Attacker can now overwrite any variable by posting the right keys

function renderTemplate(string $template, array $vars): string {
    extract($vars);
    ob_start();
    include $template;
    return ob_get_clean();
}

class Controller {
    public function show(array $params): void {
        extract($params, EXTR_OVERWRITE);
        // Even EXTR_OVERWRITE flag doesn't make this safe
        $this->render('view.php', compact('id', 'name', 'email'));
    }
}
```

### Good Code (Fix)
```php
<?php
function processForm(array $data): void {
    $name = $data['name'] ?? '';
    $role = $data['role'] ?? 'guest';
    echo "Hello, {$name}! Your role is {$role}.";
}

// Access user input explicitly
$username = $_POST['username'] ?? '';
$email = $_POST['email'] ?? '';

function renderTemplate(string $template, array $vars): string {
    // Pass vars to template via a contained scope
    $render = static function (string $_template, array $_vars): string {
        ob_start();
        // Template accesses $_vars['key'] explicitly
        include $_template;
        return ob_get_clean();
    };
    return $render($template, $vars);
}

class Controller {
    public function show(array $params): void {
        $id = $params['id'] ?? null;
        $name = $params['name'] ?? '';
        $email = $params['email'] ?? '';
        $this->render('view.php', ['id' => $id, 'name' => $name, 'email' => $email]);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_call_expression` with `name` matching `extract`
- **Detection approach**: Find `function_call_expression` nodes whose `name` child is `extract`. Every occurrence is a flag. Higher severity when the argument is a superglobal (`$_GET`, `$_POST`, `$_REQUEST`, `$_SERVER`, `$_COOKIE`) or when no flags argument is provided (defaults to `EXTR_OVERWRITE`).
- **S-expression query sketch**:
```scheme
(function_call_expression
  function: (name) @func_name
  arguments: (arguments
    (_) @first_arg)
  (#eq? @func_name "extract"))
```

### Pipeline Mapping
- **Pipeline name**: `extract_usage`
- **Pattern name**: `extract_function_call`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Business Logic in View/Template Files

### Description
Placing database queries, calculations, conditionals with side effects, or business rules directly in PHP template/view files instead of controllers or service classes. View files should only render data passed to them, not fetch or transform it. This pattern makes templates untestable and forces logic duplication.

### Bad Code (Anti-pattern)
```php
<!-- users.php (view file) -->
<?php
$pdo = new PDO('mysql:host=localhost;dbname=myapp', 'root', 'password');
$stmt = $pdo->query("SELECT * FROM users WHERE active = 1 ORDER BY created_at DESC");
$users = $stmt->fetchAll(PDO::FETCH_ASSOC);

$totalRevenue = 0;
foreach ($users as &$user) {
    $orderStmt = $pdo->prepare("SELECT SUM(total) as revenue FROM orders WHERE user_id = ?");
    $orderStmt->execute([$user['id']]);
    $user['revenue'] = $orderStmt->fetchColumn() ?: 0;
    $totalRevenue += $user['revenue'];

    if ($user['revenue'] > 10000) {
        $user['tier'] = 'gold';
    } elseif ($user['revenue'] > 1000) {
        $user['tier'] = 'silver';
    } else {
        $user['tier'] = 'bronze';
    }
}
?>
<html>
<body>
    <h1>Users (Revenue: $<?= number_format($totalRevenue, 2) ?>)</h1>
    <table>
        <?php foreach ($users as $user): ?>
        <tr class="<?= $user['tier'] ?>">
            <td><?= htmlspecialchars($user['name']) ?></td>
            <td>$<?= number_format($user['revenue'], 2) ?></td>
        </tr>
        <?php endforeach; ?>
    </table>
</body>
</html>
```

### Good Code (Fix)
```php
<?php
// UserController.php
class UserController {
    public function __construct(
        private UserRepository $userRepo,
        private RevenueService $revenueService,
    ) {}

    public function index(): void {
        $users = $this->userRepo->findActive();
        $usersWithRevenue = $this->revenueService->attachRevenue($users);
        $totalRevenue = $this->revenueService->calculateTotal($usersWithRevenue);

        include __DIR__ . '/views/users.php';
    }
}

// views/users.php (view file -- rendering only)
?>
<html>
<body>
    <h1>Users (Revenue: $<?= number_format($totalRevenue, 2) ?>)</h1>
    <table>
        <?php foreach ($usersWithRevenue as $user): ?>
        <tr class="<?= htmlspecialchars($user->tier) ?>">
            <td><?= htmlspecialchars($user->name) ?></td>
            <td>$<?= number_format($user->revenue, 2) ?></td>
        </tr>
        <?php endforeach; ?>
    </table>
</body>
</html>
```

### Tree-sitter Detection Strategy
- **Target node types**: `php_tag`, `text` (HTML content), `expression_statement` containing database calls
- **Detection approach**: Identify files that contain both HTML markup (`text` nodes or inline HTML) and PHP code performing database operations (`new PDO`, `->query()`, `->prepare()`, `->execute()`), file I/O, or complex business logic (nested loops with conditionals). Flag files where `expression_statement` nodes containing `object_creation_expression` for `PDO` or `call_expression` targeting query methods appear alongside HTML template content.
- **S-expression query sketch**:
```scheme
(program
  (text) @html_content
  (php_tag)
  (expression_statement
    (object_creation_expression
      (name) @class_name
      (#eq? @class_name "PDO"))))

(program
  (text) @html_content
  (expression_statement
    (member_call_expression
      name: (name) @method_name
      (#match? @method_name "^(query|prepare|execute)$"))))
```

### Pipeline Mapping
- **Pipeline name**: `logic_in_views`
- **Pattern name**: `database_queries_in_template`
- **Severity**: warning
- **Confidence**: high
