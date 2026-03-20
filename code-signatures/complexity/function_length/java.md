# Function Length -- Java

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```java
public OrderResult processOrder(OrderRequest request) throws OrderException {
    // Validate input
    if (request == null) {
        throw new OrderException("Request cannot be null");
    }
    if (request.getItems() == null || request.getItems().isEmpty()) {
        throw new OrderException("Order must contain at least one item");
    }
    if (request.getCustomerId() == null || request.getCustomerId().isBlank()) {
        throw new OrderException("Customer ID is required");
    }
    if (request.getShippingAddress() == null) {
        throw new OrderException("Shipping address is required");
    }
    String zipCode = request.getShippingAddress().getZipCode();
    if (zipCode == null || !zipCode.matches("\\d{5}(-\\d{4})?")) {
        throw new OrderException("Invalid ZIP code");
    }

    // Look up customer
    Customer customer = customerRepository.findById(request.getCustomerId())
        .orElseThrow(() -> new OrderException("Customer not found"));
    if (!customer.isActive()) {
        throw new OrderException("Customer account is inactive");
    }

    // Resolve items and check stock
    List<OrderLine> orderLines = new ArrayList<>();
    BigDecimal subtotal = BigDecimal.ZERO;
    for (OrderItemRequest item : request.getItems()) {
        Product product = productRepository.findBySku(item.getSku())
            .orElseThrow(() -> new OrderException("Product not found: " + item.getSku()));
        if (product.getStockQuantity() < item.getQuantity()) {
            throw new OrderException("Insufficient stock for " + product.getName());
        }
        BigDecimal lineTotal = product.getPrice().multiply(BigDecimal.valueOf(item.getQuantity()));
        subtotal = subtotal.add(lineTotal);
        orderLines.add(new OrderLine(product, item.getQuantity(), lineTotal));
    }

    // Calculate pricing
    BigDecimal discount = BigDecimal.ZERO;
    if ("GOLD".equals(customer.getTier())) {
        discount = subtotal.multiply(new BigDecimal("0.10"));
    } else if ("SILVER".equals(customer.getTier())) {
        discount = subtotal.multiply(new BigDecimal("0.05"));
    }
    BigDecimal taxableAmount = subtotal.subtract(discount);
    BigDecimal tax = taxableAmount.multiply(taxService.getRate(zipCode));
    BigDecimal total = taxableAmount.add(tax);

    // Process payment
    PaymentResult payment;
    try {
        payment = paymentGateway.charge(new PaymentRequest(
            customer.getPaymentProfileId(), total, request.getPaymentMethod()));
    } catch (PaymentException e) {
        logger.error("Payment failed for customer {}: {}", customer.getId(), e.getMessage());
        throw new OrderException("Payment processing failed", e);
    }

    // Persist order
    Order order = new Order();
    order.setCustomerId(customer.getId());
    order.setLines(orderLines);
    order.setSubtotal(subtotal);
    order.setDiscount(discount);
    order.setTax(tax);
    order.setTotal(total);
    order.setPaymentId(payment.getTransactionId());
    order.setStatus(OrderStatus.CONFIRMED);
    order.setCreatedAt(Instant.now());
    orderRepository.save(order);

    // Update inventory
    for (OrderLine line : orderLines) {
        inventoryService.decrementStock(line.getProduct().getSku(), line.getQuantity());
    }

    // Send notification
    eventPublisher.publish(new OrderCreatedEvent(order.getId(), customer.getEmail()));

    return new OrderResult(order.getId(), total, OrderStatus.CONFIRMED);
}
```

### Good Code (Fix)
```java
public OrderResult processOrder(OrderRequest request) throws OrderException {
    validateOrderRequest(request);

    Customer customer = resolveCustomer(request.getCustomerId());
    List<OrderLine> orderLines = resolveOrderLines(request.getItems());
    BigDecimal subtotal = calculateSubtotal(orderLines);
    PricingResult pricing = calculatePricing(subtotal, customer, request.getShippingAddress().getZipCode());

    PaymentResult payment = chargeCustomer(customer, pricing.getTotal(), request.getPaymentMethod());
    Order order = persistOrder(customer, orderLines, pricing, payment);

    updateInventory(orderLines);
    eventPublisher.publish(new OrderCreatedEvent(order.getId(), customer.getEmail()));

    return new OrderResult(order.getId(), pricing.getTotal(), OrderStatus.CONFIRMED);
}

private void validateOrderRequest(OrderRequest request) throws OrderException {
    if (request == null) throw new OrderException("Request cannot be null");
    if (request.getItems() == null || request.getItems().isEmpty()) {
        throw new OrderException("Order must contain at least one item");
    }
    if (request.getCustomerId() == null || request.getCustomerId().isBlank()) {
        throw new OrderException("Customer ID is required");
    }
    if (request.getShippingAddress() == null) {
        throw new OrderException("Shipping address is required");
    }
}

private Customer resolveCustomer(String customerId) throws OrderException {
    Customer customer = customerRepository.findById(customerId)
        .orElseThrow(() -> new OrderException("Customer not found"));
    if (!customer.isActive()) throw new OrderException("Customer account is inactive");
    return customer;
}

private List<OrderLine> resolveOrderLines(List<OrderItemRequest> items) throws OrderException {
    List<OrderLine> lines = new ArrayList<>();
    for (OrderItemRequest item : items) {
        Product product = productRepository.findBySku(item.getSku())
            .orElseThrow(() -> new OrderException("Product not found: " + item.getSku()));
        if (product.getStockQuantity() < item.getQuantity()) {
            throw new OrderException("Insufficient stock for " + product.getName());
        }
        BigDecimal lineTotal = product.getPrice().multiply(BigDecimal.valueOf(item.getQuantity()));
        lines.add(new OrderLine(product, item.getQuantity(), lineTotal));
    }
    return lines;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `constructor_declaration`
- **Detection approach**: Count lines between method body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    name: (identifier) @func.name
    body: (block) @func.body)

  (constructor_declaration
    name: (identifier) @func.name
    body: (constructor_body) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single Spring `@RequestMapping`/`@PostMapping` controller method that contains all business logic inline -- request validation, service logic, persistence, response building -- instead of delegating to service classes.

### Bad Code (Anti-pattern)
```java
@PostMapping("/api/users")
public ResponseEntity<?> createUser(@RequestBody Map<String, Object> body) {
    String email = (String) body.get("email");
    if (email == null || email.isBlank() || !email.contains("@")) {
        return ResponseEntity.badRequest().body(Map.of("error", "Invalid email"));
    }
    email = email.trim().toLowerCase();
    String password = (String) body.get("password");
    if (password == null || password.length() < 8) {
        return ResponseEntity.badRequest().body(Map.of("error", "Password too short"));
    }
    String name = (String) body.get("name");
    if (name == null || name.trim().length() < 2) {
        return ResponseEntity.badRequest().body(Map.of("error", "Name too short"));
    }
    name = name.trim();
    Optional<User> existing = userRepository.findByEmail(email);
    if (existing.isPresent()) {
        return ResponseEntity.status(HttpStatus.CONFLICT)
            .body(Map.of("error", "Email already registered"));
    }
    String hashedPassword = passwordEncoder.encode(password);
    User user = new User();
    user.setEmail(email);
    user.setPasswordHash(hashedPassword);
    user.setName(name);
    user.setRole("USER");
    user.setVerified(false);
    user.setCreatedAt(Instant.now());
    userRepository.save(user);
    String token = jwtService.generateToken(user.getId(), Duration.ofHours(24));
    String verificationUrl = baseUrl + "/verify?token=" + token;
    SimpleMailMessage message = new SimpleMailMessage();
    message.setTo(user.getEmail());
    message.setSubject("Verify your account");
    message.setText("Hi " + user.getName() + ", click here to verify: " + verificationUrl);
    mailSender.send(message);
    AuditLog audit = new AuditLog();
    audit.setAction("USER_CREATED");
    audit.setUserId(user.getId());
    audit.setTimestamp(Instant.now());
    auditRepository.save(audit);
    Map<String, Object> response = new LinkedHashMap<>();
    response.put("id", user.getId());
    response.put("email", user.getEmail());
    response.put("name", user.getName());
    response.put("message", "Registration successful. Check your email.");
    return ResponseEntity.status(HttpStatus.CREATED).body(response);
}
```

### Good Code (Fix)
```java
@PostMapping("/api/users")
public ResponseEntity<UserResponse> createUser(@RequestBody @Valid CreateUserRequest request) {
    UserDto user = userService.register(request);
    return ResponseEntity.status(HttpStatus.CREATED).body(UserResponse.from(user));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`
- **Detection approach**: Count lines between method body opening and closing braces. Flag when line count exceeds 40. Methods annotated with `@RequestMapping`, `@PostMapping`, `@GetMapping`, etc. are detected the same way; the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    (modifiers
      (marker_annotation
        name: (identifier) @annotation.name))
    name: (identifier) @func.name
    body: (block) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
