# Path Traversal -- JavaScript

## Overview
Path traversal vulnerabilities occur when user-supplied input is used to construct file paths without proper validation, allowing attackers to access files outside the intended directory by injecting sequences like `../` or `..\\`.

## Why It's a Security Concern
An attacker can read, write, or delete arbitrary files on the server by crafting paths that escape the intended base directory. This can lead to source code disclosure, configuration file leaks (database credentials, API keys), or remote code execution if writable paths are exploited.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Node.js fs, path, Express, Koa, Fastify

---

## Pattern 1: User Input in File Path

### Description
Concatenating user-supplied filename or path with a base directory using `path.join()` without resolving the canonical path and verifying it remains within the intended directory.

### Bad Code (Anti-pattern)
```javascript
const path = require('path');
const fs = require('fs');

app.get('/files/:name', (req, res) => {
  const filePath = path.join(__dirname, 'uploads', req.params.name);
  const content = fs.readFileSync(filePath, 'utf8');
  res.send(content);
});
```

### Good Code (Fix)
```javascript
const path = require('path');
const fs = require('fs');

app.get('/files/:name', (req, res) => {
  const baseDir = path.resolve(__dirname, 'uploads');
  const filePath = path.resolve(baseDir, req.params.name);
  if (!filePath.startsWith(baseDir + path.sep)) {
    return res.status(403).send('Forbidden');
  }
  const content = fs.readFileSync(filePath, 'utf8');
  res.send(content);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `member_expression`, `property_identifier`, `string`
- **Detection approach**: Find `call_expression` nodes where the callee is `path.join` and one of the arguments originates from user input (e.g., `req.params`, `req.query`, `req.body`). Flag when there is no subsequent `path.resolve()` + `.startsWith()` check on the result before passing it to `fs` methods.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  arguments: (arguments
    (_)
    (member_expression) @user_input))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `user_input_in_file_path`
- **Severity**: error
- **Confidence**: high

---

## Pattern 2: Directory Traversal via ../

### Description
Accepting file paths that contain `../` or `..\\` sequences without rejection or sanitization, allowing attackers to escape the intended directory.

### Bad Code (Anti-pattern)
```javascript
const fs = require('fs');

app.get('/download', (req, res) => {
  const filename = req.query.file;
  // No check for ".." — attacker sends ?file=../../../etc/passwd
  const content = fs.readFileSync(`./public/${filename}`, 'utf8');
  res.send(content);
});
```

### Good Code (Fix)
```javascript
const path = require('path');
const fs = require('fs');

app.get('/download', (req, res) => {
  const filename = req.query.file;
  if (filename.includes('..')) {
    return res.status(400).send('Invalid filename');
  }
  const baseDir = path.resolve('./public');
  const filePath = path.resolve(baseDir, filename);
  if (!filePath.startsWith(baseDir + path.sep)) {
    return res.status(403).send('Forbidden');
  }
  const content = fs.readFileSync(filePath, 'utf8');
  res.send(content);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `template_string`, `binary_expression`, `string`
- **Detection approach**: Find `call_expression` nodes calling `fs.readFileSync`, `fs.readFile`, `fs.createReadStream`, or similar, where the path argument is a template literal or string concatenation that includes a variable derived from user input, and there is no preceding check for `..` via `.includes('..')` or a regex test.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (member_expression
    object: (identifier) @fs_obj
    property: (property_identifier) @fs_method)
  arguments: (arguments
    (template_string
      (template_substitution
        (identifier) @user_var))))
```

### Pipeline Mapping
- **Pipeline name**: `path_traversal`
- **Pattern name**: `directory_traversal_dotdot`
- **Severity**: error
- **Confidence**: high
