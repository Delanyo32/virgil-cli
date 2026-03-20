# Function Length -- JavaScript/TypeScript

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```javascript
async function processOrder(req, res) {
  // Validate input
  const { items, customerId, shippingAddress, paymentMethod } = req.body;
  if (!items || !Array.isArray(items) || items.length === 0) {
    return res.status(400).json({ error: "Items are required" });
  }
  if (!customerId || typeof customerId !== "string") {
    return res.status(400).json({ error: "Valid customer ID is required" });
  }
  if (!shippingAddress || !shippingAddress.street || !shippingAddress.city) {
    return res.status(400).json({ error: "Complete shipping address required" });
  }
  if (!paymentMethod || !["credit", "debit", "paypal"].includes(paymentMethod)) {
    return res.status(400).json({ error: "Valid payment method required" });
  }

  // Look up customer
  const customer = await db.customers.findById(customerId);
  if (!customer) {
    return res.status(404).json({ error: "Customer not found" });
  }
  if (customer.suspended) {
    return res.status(403).json({ error: "Customer account suspended" });
  }

  // Calculate totals
  let subtotal = 0;
  const enrichedItems = [];
  for (const item of items) {
    const product = await db.products.findById(item.productId);
    if (!product) {
      return res.status(404).json({ error: `Product ${item.productId} not found` });
    }
    if (product.stock < item.quantity) {
      return res.status(409).json({ error: `Insufficient stock for ${product.name}` });
    }
    const lineTotal = product.price * item.quantity;
    subtotal += lineTotal;
    enrichedItems.push({ ...item, product, lineTotal });
  }

  // Apply discounts
  let discount = 0;
  if (customer.tier === "gold") {
    discount = subtotal * 0.1;
  } else if (customer.tier === "silver") {
    discount = subtotal * 0.05;
  }
  if (items.length > 5) {
    discount += subtotal * 0.02;
  }
  const tax = (subtotal - discount) * 0.08;
  const total = subtotal - discount + tax;

  // Process payment
  let paymentResult;
  try {
    paymentResult = await paymentGateway.charge({
      amount: total,
      method: paymentMethod,
      customerId: customer.paymentProfileId,
    });
  } catch (err) {
    logger.error("Payment failed", err);
    return res.status(502).json({ error: "Payment processing failed" });
  }

  // Create order record
  const order = await db.orders.create({
    customerId,
    items: enrichedItems,
    subtotal,
    discount,
    tax,
    total,
    paymentId: paymentResult.id,
    shippingAddress,
    status: "confirmed",
    createdAt: new Date(),
  });

  // Update inventory
  for (const item of enrichedItems) {
    await db.products.updateStock(item.productId, -item.quantity);
  }

  // Send confirmation
  await emailService.send({
    to: customer.email,
    subject: `Order ${order.id} Confirmed`,
    template: "order-confirmation",
    data: { order, customer },
  });

  return res.status(201).json({ orderId: order.id, total });
}
```

### Good Code (Fix)
```javascript
function validateOrderInput(body) {
  const { items, customerId, shippingAddress, paymentMethod } = body;
  const errors = [];
  if (!items || !Array.isArray(items) || items.length === 0) {
    errors.push("Items are required");
  }
  if (!customerId || typeof customerId !== "string") {
    errors.push("Valid customer ID is required");
  }
  if (!shippingAddress || !shippingAddress.street || !shippingAddress.city) {
    errors.push("Complete shipping address required");
  }
  if (!paymentMethod || !["credit", "debit", "paypal"].includes(paymentMethod)) {
    errors.push("Valid payment method required");
  }
  return errors;
}

async function resolveCustomer(customerId) {
  const customer = await db.customers.findById(customerId);
  if (!customer) throw new AppError(404, "Customer not found");
  if (customer.suspended) throw new AppError(403, "Customer account suspended");
  return customer;
}

async function enrichItems(items) {
  const enriched = [];
  let subtotal = 0;
  for (const item of items) {
    const product = await db.products.findById(item.productId);
    if (!product) throw new AppError(404, `Product ${item.productId} not found`);
    if (product.stock < item.quantity) {
      throw new AppError(409, `Insufficient stock for ${product.name}`);
    }
    const lineTotal = product.price * item.quantity;
    subtotal += lineTotal;
    enriched.push({ ...item, product, lineTotal });
  }
  return { enriched, subtotal };
}

function calculateTotals(subtotal, customerTier, itemCount) {
  let discount = 0;
  if (customerTier === "gold") discount = subtotal * 0.1;
  else if (customerTier === "silver") discount = subtotal * 0.05;
  if (itemCount > 5) discount += subtotal * 0.02;
  const tax = (subtotal - discount) * 0.08;
  return { discount, tax, total: subtotal - discount + tax };
}

async function processOrder(req, res) {
  const errors = validateOrderInput(req.body);
  if (errors.length) return res.status(400).json({ errors });

  const { items, customerId, shippingAddress, paymentMethod } = req.body;
  const customer = await resolveCustomer(customerId);
  const { enriched, subtotal } = await enrichItems(items);
  const totals = calculateTotals(subtotal, customer.tier, items.length);

  const paymentResult = await paymentGateway.charge({
    amount: totals.total, method: paymentMethod,
    customerId: customer.paymentProfileId,
  });

  const order = await db.orders.create({
    customerId, items: enriched, ...totals,
    paymentId: paymentResult.id, shippingAddress,
    status: "confirmed", createdAt: new Date(),
  });

  await Promise.all([
    ...enriched.map(i => db.products.updateStock(i.productId, -i.quantity)),
    emailService.send({
      to: customer.email, subject: `Order ${order.id} Confirmed`,
      template: "order-confirmation", data: { order, customer },
    }),
  ]);

  return res.status(201).json({ orderId: order.id, total: totals.total });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_definition`, `arrow_function`, `function`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (function_declaration
    name: (identifier) @func.name
    body: (statement_block) @func.body)

  (method_definition
    name: (property_identifier) @func.name
    body: (statement_block) @func.body)

  (variable_declarator
    name: (identifier) @func.name
    value: (arrow_function
      body: (statement_block) @func.body))
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single Express/Koa/Fastify route handler that contains all business logic inline -- parsing, validation, database queries, response formatting -- instead of delegating to service functions.

### Bad Code (Anti-pattern)
```javascript
app.post("/api/users/register", async (req, res) => {
  const { email, password, name, role } = req.body;
  if (!email || !email.includes("@")) {
    return res.status(400).json({ error: "Invalid email" });
  }
  if (!password || password.length < 8) {
    return res.status(400).json({ error: "Password too short" });
  }
  if (!name || name.trim().length < 2) {
    return res.status(400).json({ error: "Name too short" });
  }
  const existing = await db.users.findOne({ email: email.toLowerCase() });
  if (existing) {
    return res.status(409).json({ error: "Email already registered" });
  }
  const salt = await bcrypt.genSalt(12);
  const hashedPassword = await bcrypt.hash(password, salt);
  const user = await db.users.create({
    email: email.toLowerCase(),
    password: hashedPassword,
    name: name.trim(),
    role: role || "user",
    verified: false,
    createdAt: new Date(),
  });
  const token = jwt.sign({ userId: user.id }, process.env.JWT_SECRET, {
    expiresIn: "24h",
  });
  const verificationLink = `${process.env.BASE_URL}/verify?token=${token}`;
  await transporter.sendMail({
    from: process.env.EMAIL_FROM,
    to: user.email,
    subject: "Verify your account",
    html: `<p>Hi ${user.name},</p><p>Click <a href="${verificationLink}">here</a> to verify.</p>`,
  });
  await db.auditLog.create({
    action: "user_registered",
    userId: user.id,
    ip: req.ip,
    timestamp: new Date(),
  });
  const { password: _, ...safeUser } = user.toObject();
  return res.status(201).json({
    user: safeUser,
    message: "Registration successful. Check your email to verify.",
  });
});
```

### Good Code (Fix)
```javascript
const { validateRegistration } = require("./validators/user");
const userService = require("./services/user");

app.post("/api/users/register", async (req, res) => {
  const errors = validateRegistration(req.body);
  if (errors.length) {
    return res.status(400).json({ errors });
  }

  const result = await userService.register(req.body, req.ip);
  return res.status(201).json(result);
});
```

### Tree-sitter Detection Strategy
- **Target node types**: `arrow_function`, `function_declaration`, `method_definition`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40. Handler functions are identified the same way as regular functions -- the pattern name differentiates them in reporting, but the detection mechanism is identical line counting.
- **S-expression query sketch**:
  ```scheme
  (arrow_function
    body: (statement_block) @func.body)

  (call_expression
    function: (member_expression
      property: (property_identifier) @http.method)
    arguments: (arguments
      (arrow_function
        body: (statement_block) @handler.body)))
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
