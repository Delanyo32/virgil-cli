# Comment Ratio -- Java

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .java
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```java
public List<Invoice> generateInvoices(List<Order> orders, TaxService taxService) {
    List<Invoice> invoices = new ArrayList<>();
    Map<String, List<Order>> grouped = new HashMap<>();
    for (Order order : orders) {
        String key = order.getCustomerId() + "-" + order.getCurrency();
        grouped.computeIfAbsent(key, k -> new ArrayList<>()).add(order);
    }
    for (Map.Entry<String, List<Order>> entry : grouped.entrySet()) {
        List<Order> customerOrders = entry.getValue();
        BigDecimal subtotal = BigDecimal.ZERO;
        for (Order order : customerOrders) {
            if (order.getStatus() == OrderStatus.CANCELLED) {
                continue;
            }
            for (LineItem item : order.getLineItems()) {
                BigDecimal lineTotal = item.getPrice().multiply(BigDecimal.valueOf(item.getQuantity()));
                if (item.getDiscount() != null && item.getDiscount().compareTo(BigDecimal.ZERO) > 0) {
                    lineTotal = lineTotal.subtract(lineTotal.multiply(item.getDiscount()));
                }
                subtotal = subtotal.add(lineTotal);
            }
        }
        BigDecimal tax = taxService.calculate(customerOrders.get(0).getCustomerId(), subtotal);
        if (tax == null) {
            tax = subtotal.multiply(new BigDecimal("0.21"));
        }
        Invoice invoice = new Invoice(
            customerOrders.get(0).getCustomerId(),
            subtotal,
            tax,
            subtotal.add(tax),
            customerOrders.get(0).getCurrency()
        );
        invoices.add(invoice);
    }
    return invoices;
}
```

### Good Code (Fix)
```java
/**
 * Consolidates orders into per-customer, per-currency invoices with tax calculation.
 * Orders from the same customer in different currencies produce separate invoices
 * to comply with multi-currency accounting regulations.
 */
public List<Invoice> generateInvoices(List<Order> orders, TaxService taxService) {
    List<Invoice> invoices = new ArrayList<>();

    // Group by customer+currency so each invoice is single-currency
    Map<String, List<Order>> grouped = new HashMap<>();
    for (Order order : orders) {
        String key = order.getCustomerId() + "-" + order.getCurrency();
        grouped.computeIfAbsent(key, k -> new ArrayList<>()).add(order);
    }

    for (Map.Entry<String, List<Order>> entry : grouped.entrySet()) {
        List<Order> customerOrders = entry.getValue();
        BigDecimal subtotal = BigDecimal.ZERO;

        for (Order order : customerOrders) {
            if (order.getStatus() == OrderStatus.CANCELLED) {
                continue;
            }
            for (LineItem item : order.getLineItems()) {
                BigDecimal lineTotal = item.getPrice().multiply(BigDecimal.valueOf(item.getQuantity()));
                if (item.getDiscount() != null && item.getDiscount().compareTo(BigDecimal.ZERO) > 0) {
                    lineTotal = lineTotal.subtract(lineTotal.multiply(item.getDiscount()));
                }
                subtotal = subtotal.add(lineTotal);
            }
        }

        // Tax service may return null for unregistered regions;
        // fall back to standard 21% EU VAT rate
        BigDecimal tax = taxService.calculate(customerOrders.get(0).getCustomerId(), subtotal);
        if (tax == null) {
            tax = subtotal.multiply(new BigDecimal("0.21"));
        }

        Invoice invoice = new Invoice(
            customerOrders.get(0).getCustomerId(),
            subtotal,
            tax,
            subtotal.add(tax),
            customerOrders.get(0).getCurrency()
        );
        invoices.add(invoice);
    }
    return invoices;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `constructor_declaration` for function bodies; `line_comment` for `//`; `block_comment` for `/* */`; Javadoc shows as `block_comment` starting with `/**`
- **Detection approach**: Count comment lines and code lines within a method body. Calculate ratio. Flag methods with CC > 5 and comment ratio below threshold. Consider Javadoc above the method signature as part of the method's documentation.
- **S-expression query sketch**:
  ```scheme
  ;; Capture method body and any comments within it
  (method_declaration
    body: (block) @function.body)

  (constructor_declaration
    body: (constructor_body) @function.body)

  (line_comment) @comment
  (block_comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```java
public void processPayment(Payment payment) {
    // Get the amount
    BigDecimal amount = payment.getAmount();

    // Check if amount is less than or equal to zero
    if (amount.compareTo(BigDecimal.ZERO) <= 0) {
        // Throw illegal argument exception
        throw new IllegalArgumentException("Invalid amount");
    }

    // Create a new transaction
    Transaction tx = new Transaction(payment);

    // Set the timestamp
    tx.setTimestamp(Instant.now());

    // Save the transaction
    transactionRepository.save(tx);

    // Log the payment
    logger.info("Payment processed: {}", payment.getId());
}
```

### Good Code (Fix)
```java
public void processPayment(Payment payment) {
    BigDecimal amount = payment.getAmount();
    if (amount.compareTo(BigDecimal.ZERO) <= 0) {
        throw new IllegalArgumentException("Invalid amount");
    }

    Transaction tx = new Transaction(payment);
    tx.setTimestamp(Instant.now());

    // Persist before sending confirmation -- downstream listeners
    // rely on the transaction existing in the DB
    transactionRepository.save(tx);
    logger.info("Payment processed: {}", payment.getId());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `line_comment`, `block_comment` adjacent to `local_variable_declaration`, `expression_statement`, `return_statement`, `if_statement`, `throw_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (block
    (line_comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
