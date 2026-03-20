# Duplicate Code -- Java

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .java
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more methods with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared method with parameters. Common in service layer methods across controllers.

### Bad Code (Anti-pattern)
```java
public class OrderController {

    public ResponseEntity<OrderResponse> createUserOrder(UserOrderRequest request) {
        if (request.getName() == null || request.getEmail() == null) {
            throw new ValidationException("Name and email are required");
        }
        String normalizedName = request.getName().trim().toLowerCase();
        UserOrder order = new UserOrder();
        order.setName(normalizedName);
        order.setEmail(request.getEmail());
        order.setAmount(request.getQuantity() * request.getPrice());
        order.setTax(order.getAmount() * 0.08);
        order.setCreatedAt(Instant.now());
        userOrderRepository.save(order);
        emailService.send(request.getEmail(), "Order confirmed", formatReceipt(order));
        return ResponseEntity.status(HttpStatus.CREATED).body(toResponse(order));
    }

    public ResponseEntity<OrderResponse> createAdminOrder(AdminOrderRequest request) {
        if (request.getName() == null || request.getEmail() == null) {
            throw new ValidationException("Name and email are required");
        }
        String normalizedName = request.getName().trim().toLowerCase();
        AdminOrder order = new AdminOrder();
        order.setName(normalizedName);
        order.setEmail(request.getEmail());
        order.setAmount(request.getQuantity() * request.getPrice());
        order.setTax(order.getAmount() * 0.08);
        order.setCreatedAt(Instant.now());
        adminOrderRepository.save(order);
        emailService.send(request.getEmail(), "Order confirmed", formatReceipt(order));
        return ResponseEntity.status(HttpStatus.CREATED).body(toResponse(order));
    }
}
```

### Good Code (Fix)
```java
public class OrderController {

    private <T extends BaseOrder> ResponseEntity<OrderResponse> createOrder(
            OrderRequest request, T order, JpaRepository<T, Long> repository) {
        if (request.getName() == null || request.getEmail() == null) {
            throw new ValidationException("Name and email are required");
        }
        String normalizedName = request.getName().trim().toLowerCase();
        order.setName(normalizedName);
        order.setEmail(request.getEmail());
        order.setAmount(request.getQuantity() * request.getPrice());
        order.setTax(order.getAmount() * 0.08);
        order.setCreatedAt(Instant.now());
        repository.save(order);
        emailService.send(request.getEmail(), "Order confirmed", formatReceipt(order));
        return ResponseEntity.status(HttpStatus.CREATED).body(toResponse(order));
    }

    public ResponseEntity<OrderResponse> createUserOrder(UserOrderRequest request) {
        return createOrder(request, new UserOrder(), userOrderRepository);
    }

    public ResponseEntity<OrderResponse> createAdminOrder(AdminOrderRequest request) {
        return createOrder(request, new AdminOrder(), adminOrderRepository);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `block`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(method_declaration
  name: (identifier) @func_name
  body: (block) @func_body)
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a method or across methods in the same class, often due to copy-paste during development. Common in duplicated DAO patterns and repeated service method structures.

### Bad Code (Anti-pattern)
```java
public class UserService {

    public UserDTO findAndEnrich(Long userId) {
        User user = userRepository.findById(userId)
            .orElseThrow(() -> new NotFoundException("User not found: " + userId));
        UserDTO dto = new UserDTO();
        dto.setId(user.getId());
        dto.setName(user.getName());
        dto.setEmail(user.getEmail());
        List<Order> orders = orderRepository.findByUserId(userId);
        dto.setOrderCount(orders.size());
        dto.setTotalSpent(orders.stream().mapToDouble(Order::getAmount).sum());
        auditLog.record("user_view", userId);
        return dto;
    }

    public UserDTO findAndEnrichForAdmin(Long userId) {
        User user = userRepository.findById(userId)
            .orElseThrow(() -> new NotFoundException("User not found: " + userId));
        UserDTO dto = new UserDTO();
        dto.setId(user.getId());
        dto.setName(user.getName());
        dto.setEmail(user.getEmail());
        List<Order> orders = orderRepository.findByUserId(userId);
        dto.setOrderCount(orders.size());
        dto.setTotalSpent(orders.stream().mapToDouble(Order::getAmount).sum());
        auditLog.record("admin_user_view", userId);
        return dto;
    }
}
```

### Good Code (Fix)
```java
public class UserService {

    private UserDTO buildEnrichedDTO(Long userId) {
        User user = userRepository.findById(userId)
            .orElseThrow(() -> new NotFoundException("User not found: " + userId));
        UserDTO dto = new UserDTO();
        dto.setId(user.getId());
        dto.setName(user.getName());
        dto.setEmail(user.getEmail());
        List<Order> orders = orderRepository.findByUserId(userId);
        dto.setOrderCount(orders.size());
        dto.setTotalSpent(orders.stream().mapToDouble(Order::getAmount).sum());
        return dto;
    }

    public UserDTO findAndEnrich(Long userId) {
        UserDTO dto = buildEnrichedDTO(userId);
        auditLog.record("user_view", userId);
        return dto;
    }

    public UserDTO findAndEnrichForAdmin(Long userId) {
        UserDTO dto = buildEnrichedDTO(userId);
        auditLog.record("admin_user_view", userId);
        return dto;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `block`, `local_variable_declaration`, `expression_statement`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across method bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(block
  (_) @stmt)

(method_declaration
  body: (block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
