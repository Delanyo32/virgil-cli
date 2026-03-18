# Injection -- JavaScript

## Overview
Injection vulnerabilities occur when untrusted user input is incorporated into queries, commands, or executable code without proper sanitization or parameterization. In JavaScript/TypeScript applications, the most critical injection vectors are SQL injection via string concatenation, command injection through `child_process`, and DOM-based XSS via unsafe DOM manipulation APIs.

## Why It's a Security Concern
Injection attacks allow adversaries to execute arbitrary SQL queries, operating system commands, or client-side scripts in the context of the application. SQL injection can lead to full database compromise; command injection can grant shell access to the server; XSS can steal session tokens, impersonate users, and exfiltrate sensitive data. These are consistently ranked among the top web application vulnerabilities (OWASP Top 10).

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: node-postgres, mysql2, knex, sequelize, child_process, React (dangerouslySetInnerHTML), vanilla DOM API

---

## Pattern 1: SQL Injection via String Concatenation

### Description
Building SQL query strings by concatenating or interpolating user-supplied values directly, instead of using parameterized queries or prepared statements.

### Bad Code (Anti-pattern)
```typescript
import { Pool } from 'pg';

async function getUser(pool: Pool, userId: string) {
  const query = "SELECT * FROM users WHERE id = '" + userId + "'";
  const result = await pool.query(query);
  return result.rows[0];
}
```

### Good Code (Fix)
```typescript
import { Pool } from 'pg';

async function getUser(pool: Pool, userId: string) {
  const query = "SELECT * FROM users WHERE id = $1";
  const result = await pool.query(query, [userId]);
  return result.rows[0];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `template_string`, `binary_expression`, `string`
- **Detection approach**: Find `call_expression` nodes where the callee is `.query()`, `.execute()`, or `.raw()` and the first argument is a `binary_expression` (string concatenation) or a `template_string` containing embedded expressions. The presence of SQL keywords (`SELECT`, `INSERT`, `UPDATE`, `DELETE`) in the string confirms it as a SQL query.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    property: (property_identifier) @method)
  arguments: (arguments
    (binary_expression
      left: (string) @sql_fragment
      right: (_) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `sql_string_concat`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Command Injection via child_process.exec

### Description
Passing user-controlled input to `child_process.exec()` or `child_process.execSync()`, which invokes a system shell and interprets shell metacharacters. Attackers can inject arbitrary commands using characters like `;`, `&&`, `|`, or backticks.

### Bad Code (Anti-pattern)
```typescript
import { exec } from 'child_process';

function convertFile(userFilename: string) {
  exec(`convert ${userFilename} output.png`, (error, stdout, stderr) => {
    if (error) throw error;
  });
}
```

### Good Code (Fix)
```typescript
import { execFile } from 'child_process';

function convertFile(userFilename: string) {
  execFile('convert', [userFilename, 'output.png'], (error, stdout, stderr) => {
    if (error) throw error;
  });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `import_statement`, `template_string`, `binary_expression`
- **Detection approach**: Find `call_expression` nodes calling `exec` or `execSync` (imported from `child_process`) where the first argument is a `template_string` or `binary_expression` containing a variable. Distinguish from `execFile` which takes an argument array and does not invoke a shell.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (identifier) @func_name
  arguments: (arguments
    (template_string
      (template_substitution) @user_input)))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `command_injection_exec`
- **Severity**: error
- **Confidence**: high

---

## Pattern 3: DOM XSS via innerHTML / document.write

### Description
Assigning user-controlled data to `innerHTML`, `outerHTML`, or passing it to `document.write()` without sanitization. This allows attackers to inject arbitrary HTML and JavaScript that executes in the victim's browser context.

### Bad Code (Anti-pattern)
```javascript
function renderComment(comment) {
  const container = document.getElementById('comments');
  container.innerHTML = `<div class="comment">${comment.body}</div>`;
}

function displaySearch(query) {
  document.write('<h1>Results for: ' + query + '</h1>');
}
```

### Good Code (Fix)
```javascript
function renderComment(comment) {
  const container = document.getElementById('comments');
  const div = document.createElement('div');
  div.className = 'comment';
  div.textContent = comment.body;
  container.appendChild(div);
}

function displaySearch(query) {
  const heading = document.createElement('h1');
  heading.textContent = `Results for: ${query}`;
  document.body.appendChild(heading);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `assignment_expression`, `member_expression`, `call_expression`, `template_string`, `binary_expression`
- **Detection approach**: Find `assignment_expression` nodes where the left side is a `member_expression` with property `innerHTML` or `outerHTML` and the right side contains a `template_string` with substitutions or a `binary_expression` with a non-literal operand. Also find `call_expression` nodes calling `document.write` or `document.writeln` with non-literal arguments.
- **S-expression query sketch**:
```scheme
(assignment_expression
  left: (member_expression
    property: (property_identifier) @prop)
  right: (template_string
    (template_substitution) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `injection`
- **Pattern name**: `dom_xss_innerhtml`
- **Severity**: error
- **Confidence**: high
