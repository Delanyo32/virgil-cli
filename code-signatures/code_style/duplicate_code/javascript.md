# Duplicate Code -- JavaScript

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters.

### Bad Code (Anti-pattern)
```javascript
function processUserOrder(order) {
  const validated = validateFields(order, ['id', 'product', 'quantity']);
  if (!validated) {
    logger.error('Invalid order fields');
    return { success: false, error: 'Validation failed' };
  }
  const total = order.quantity * order.price;
  const tax = total * 0.08;
  const record = { ...order, total, tax, processedAt: new Date() };
  db.orders.insert(record);
  emailService.send(order.email, 'Order confirmed', formatReceipt(record));
  return { success: true, data: record };
}

function processAdminOrder(order) {
  const validated = validateFields(order, ['id', 'product', 'quantity']);
  if (!validated) {
    logger.error('Invalid order fields');
    return { success: false, error: 'Validation failed' };
  }
  const total = order.quantity * order.price;
  const tax = total * 0.08;
  const record = { ...order, total, tax, processedAt: new Date() };
  db.orders.insert(record);
  emailService.send(order.email, 'Order confirmed', formatReceipt(record));
  return { success: true, data: record };
}
```

### Good Code (Fix)
```javascript
function processOrder(order, role = 'user') {
  const validated = validateFields(order, ['id', 'product', 'quantity']);
  if (!validated) {
    logger.error(`Invalid order fields for ${role}`);
    return { success: false, error: 'Validation failed' };
  }
  const total = order.quantity * order.price;
  const tax = total * 0.08;
  const record = { ...order, total, tax, role, processedAt: new Date() };
  db.orders.insert(record);
  emailService.send(order.email, 'Order confirmed', formatReceipt(record));
  return { success: true, data: record };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `variable_declarator` (arrow functions), `method_definition`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_declaration
  name: (identifier) @func_name
  body: (statement_block) @func_body)

(variable_declarator
  name: (identifier) @func_name
  value: (arrow_function
    body: (statement_block) @func_body))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a function or across methods in the same class, often due to copy-paste during development. Common in Express route handlers and DOM manipulation code.

### Bad Code (Anti-pattern)
```javascript
app.post('/api/users', (req, res) => {
  const { name, email } = req.body;
  if (!name || !email) {
    return res.status(400).json({ error: 'Missing fields' });
  }
  const sanitizedName = name.trim().toLowerCase();
  const existing = db.users.findOne({ email });
  if (existing) {
    return res.status(409).json({ error: 'Already exists' });
  }
  const record = db.users.insert({ name: sanitizedName, email, createdAt: new Date() });
  logger.info(`Created user ${record.id}`);
  res.status(201).json(record);
});

app.post('/api/vendors', (req, res) => {
  const { name, email } = req.body;
  if (!name || !email) {
    return res.status(400).json({ error: 'Missing fields' });
  }
  const sanitizedName = name.trim().toLowerCase();
  const existing = db.vendors.findOne({ email });
  if (existing) {
    return res.status(409).json({ error: 'Already exists' });
  }
  const record = db.vendors.insert({ name: sanitizedName, email, createdAt: new Date() });
  logger.info(`Created vendor ${record.id}`);
  res.status(201).json(record);
});
```

### Good Code (Fix)
```javascript
function createEntity(collection, entityType) {
  return (req, res) => {
    const { name, email } = req.body;
    if (!name || !email) {
      return res.status(400).json({ error: 'Missing fields' });
    }
    const sanitizedName = name.trim().toLowerCase();
    const existing = collection.findOne({ email });
    if (existing) {
      return res.status(409).json({ error: 'Already exists' });
    }
    const record = collection.insert({ name: sanitizedName, email, createdAt: new Date() });
    logger.info(`Created ${entityType} ${record.id}`);
    res.status(201).json(record);
  };
}

app.post('/api/users', createEntity(db.users, 'user'));
app.post('/api/vendors', createEntity(db.vendors, 'vendor'));
```

### Tree-sitter Detection Strategy
- **Target node types**: `statement_block`, `expression_statement`, `variable_declaration`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(statement_block
  (_) @stmt)

(arrow_function
  body: (statement_block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
