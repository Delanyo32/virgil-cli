# God Objects -- Java

## Overview
God objects are classes that accumulate too many responsibilities — too many fields, too many methods, or too many lines — violating the Single Responsibility Principle. In Java, this commonly appears as monolithic service classes, oversized Spring controllers with 20+ endpoints, or utility classes that become dumping grounds for unrelated functionality.

## Why It's a Tech Debt Concern
God classes in Java become bottlenecks for development as every feature change risks touching the same file, causing frequent merge conflicts. Unit testing requires extensive mocking since responsibilities are entangled, leading to fragile tests that break with unrelated changes. The cognitive load of navigating a 2000+ line class with 40+ methods discourages thorough code review and increases the likelihood of introducing regressions.

## Applicability
- **Relevance**: high (Java's class-centric design makes god classes a pervasive anti-pattern)
- **Languages covered**: `.java`
- **Frameworks/libraries**: Spring Boot (@Controller/@Service with 20+ endpoints/methods), Jakarta EE (oversized EJBs), Android (god Activities/Fragments)

---

## Pattern 1: Oversized Class

### Description
A class with 30+ methods, 15+ fields, or exceeding 500 lines of code. The class handles multiple unrelated concerns such as user management, authentication, email, caching, and reporting all in one type.

### Bad Code (Anti-pattern)
```java
@Service
public class UserService {
    @Autowired private UserRepository userRepository;
    @Autowired private PasswordEncoder passwordEncoder;
    @Autowired private JavaMailSender mailSender;
    @Autowired private RedisTemplate<String, Object> redisTemplate;
    @Autowired private AmazonS3 s3Client;
    @Autowired private ElasticsearchClient searchClient;
    @Autowired private MeterRegistry meterRegistry;
    @Autowired private RabbitTemplate rabbitTemplate;
    @Autowired private Validator validator;
    @Autowired private AuditLogRepository auditLogRepository;
    @Autowired private SessionRepository sessionRepository;
    @Autowired private FeatureFlagService featureFlagService;
    @Autowired private RateLimiterService rateLimiterService;
    @Autowired private TwoFactorAuthService twoFactorAuthService;
    @Autowired private OAuthClientService oAuthClientService;
    @Autowired private NotificationService notificationService;

    public User createUser(CreateUserRequest request) { /* ... */ }
    public User updateUser(UUID id, UpdateUserRequest request) { /* ... */ }
    public void deleteUser(UUID id) { /* ... */ }
    public User findUser(UUID id) { /* ... */ }
    public Page<User> listUsers(UserFilter filter, Pageable pageable) { /* ... */ }
    public List<User> searchUsers(String query) { /* ... */ }
    public void validateEmail(String email) { /* ... */ }
    public void validatePassword(String password) { /* ... */ }
    public void validateUsername(String username) { /* ... */ }
    public String hashPassword(String password) { /* ... */ }
    public boolean verifyPassword(String plain, String hashed) { /* ... */ }
    public String generateToken(User user) { /* ... */ }
    public Claims verifyToken(String token) { /* ... */ }
    public String refreshToken(String token) { /* ... */ }
    public void sendWelcomeEmail(User user) { /* ... */ }
    public void sendPasswordResetEmail(User user) { /* ... */ }
    public void sendVerificationEmail(User user) { /* ... */ }
    public String uploadAvatar(User user, MultipartFile file) { /* ... */ }
    public BufferedImage resizeAvatar(MultipartFile file, int size) { /* ... */ }
    public void cacheUser(User user) { /* ... */ }
    public void invalidateCache(UUID userId) { /* ... */ }
    public void logActivity(User user, String action) { /* ... */ }
    public boolean checkPermission(User user, String resource) { /* ... */ }
    public void trackMetric(String event, Map<String, Object> data) { /* ... */ }
    public boolean rateLimitCheck(UUID userId, String action) { /* ... */ }
    public byte[] exportUserData(User user) { /* ... */ }
    public void importUsers(MultipartFile csvFile) { /* ... */ }
    public Report generateReport(ReportFilter filter) { /* ... */ }
    public void indexUser(User user) { /* ... */ }
    public void reindexAll() { /* ... */ }
    public TwoFactorSetup setupTwoFactor(User user) { /* ... */ }
    public boolean verifyTwoFactor(User user, String code) { /* ... */ }
    public void handleOAuthCallback(String provider, String code) { /* ... */ }
    public void syncExternalProfile(User user, String provider) { /* ... */ }
}
```

### Good Code (Fix)
```java
@Service
public class UserService {
    private final UserRepository userRepository;

    public User createUser(CreateUserRequest request) { /* ... */ }
    public User updateUser(UUID id, UpdateUserRequest request) { /* ... */ }
    public void deleteUser(UUID id) { /* ... */ }
    public User findUser(UUID id) { /* ... */ }
    public Page<User> listUsers(UserFilter filter, Pageable pageable) { /* ... */ }
}

@Service
public class AuthService {
    private final PasswordEncoder passwordEncoder;
    private final JwtTokenProvider tokenProvider;

    public String hashPassword(String password) { /* ... */ }
    public boolean verifyPassword(String plain, String hashed) { /* ... */ }
    public String generateToken(User user) { /* ... */ }
    public Claims verifyToken(String token) { /* ... */ }
}

@Service
public class UserEmailService {
    private final JavaMailSender mailSender;

    public void sendWelcome(User user) { /* ... */ }
    public void sendPasswordReset(User user) { /* ... */ }
    public void sendVerification(User user) { /* ... */ }
}

@Service
public class UserSearchService {
    private final ElasticsearchClient searchClient;

    public List<User> search(String query) { /* ... */ }
    public void index(User user) { /* ... */ }
    public void reindexAll() { /* ... */ }
}

@Service
public class AvatarService {
    private final AmazonS3 s3Client;

    public String upload(User user, MultipartFile file) { /* ... */ }
    public BufferedImage resize(MultipartFile file, int size) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration` (count `field_declaration` and `method_declaration` children in `class_body`)
- **Detection approach**: Count `field_declaration` nodes and `method_declaration` nodes within a `class_body`. Flag when methods exceed 20, fields exceed 15, or total lines exceed 500. Include `constructor_declaration` in the method count.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (class_body
      (method_declaration
        name: (identifier) @method_name)))

  (class_declaration
    body: (class_body
      (field_declaration) @field))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class handling HTTP request processing, input validation, business logic, database access, and notification — a clear SRP violation. In Java, this frequently manifests as a Spring `@Controller` or `@RestController` that bypasses the service layer and performs all operations directly.

### Bad Code (Anti-pattern)
```java
@RestController
@RequestMapping("/api/orders")
public class OrderController {
    @Autowired private JdbcTemplate jdbcTemplate;
    @Autowired private JavaMailSender mailSender;
    @Autowired private RedisTemplate<String, Object> cache;
    @Autowired private MeterRegistry metrics;

    @PostMapping
    public ResponseEntity<Order> createOrder(@RequestBody CreateOrderRequest request) {
        // Validation
        if (request.getItems() == null || request.getItems().isEmpty()) {
            return ResponseEntity.badRequest().build();
        }
        for (OrderItem item : request.getItems()) {
            if (item.getQuantity() <= 0) {
                return ResponseEntity.badRequest().build();
            }
        }
        // Business logic
        BigDecimal subtotal = request.getItems().stream()
            .map(i -> i.getPrice().multiply(BigDecimal.valueOf(i.getQuantity())))
            .reduce(BigDecimal.ZERO, BigDecimal::add);
        BigDecimal tax = subtotal.multiply(new BigDecimal("0.08"));
        BigDecimal discount = calculateDiscount(request.getCoupon());
        BigDecimal total = subtotal.add(tax).subtract(discount);
        // Database
        UUID orderId = UUID.randomUUID();
        jdbcTemplate.update("INSERT INTO orders ...", orderId, total, tax);
        for (OrderItem item : request.getItems()) {
            jdbcTemplate.update("INSERT INTO order_items ...", orderId, item.getId());
            jdbcTemplate.update("UPDATE inventory SET stock = stock - ? ...", item.getQuantity());
        }
        // Email
        sendConfirmationEmail(request.getEmail(), orderId, total);
        // Metrics
        metrics.counter("orders.created").increment();
        return ResponseEntity.ok(new Order(orderId, total));
    }

    private BigDecimal calculateDiscount(String coupon) { /* ... */ }
    private void sendConfirmationEmail(String email, UUID orderId, BigDecimal total) { /* ... */ }
    @GetMapping("/{id}") public ResponseEntity<Order> getOrder(@PathVariable UUID id) { /* ... */ }
    @PutMapping("/{id}") public ResponseEntity<Order> updateOrder(@PathVariable UUID id, @RequestBody UpdateOrderRequest req) { /* ... */ }
    @DeleteMapping("/{id}") public ResponseEntity<Void> cancelOrder(@PathVariable UUID id) { /* ... */ }
    @PostMapping("/{id}/refund") public ResponseEntity<Void> refundOrder(@PathVariable UUID id) { /* ... */ }
    @GetMapping public ResponseEntity<List<Order>> listOrders(@RequestParam Map<String, String> filters) { /* ... */ }
    @GetMapping("/export") public ResponseEntity<byte[]> exportOrders(@RequestParam String format) { /* ... */ }
    @GetMapping("/{id}/invoice") public ResponseEntity<byte[]> generateInvoice(@PathVariable UUID id) { /* ... */ }
    private void validateCoupon(String code) { /* ... */ }
    private BigDecimal calculateShipping(List<OrderItem> items, Address address) { /* ... */ }
    private void notifyWarehouse(UUID orderId) { /* ... */ }
    private void updateInventory(List<OrderItem> items) { /* ... */ }
}
```

### Good Code (Fix)
```java
@RestController
@RequestMapping("/api/orders")
public class OrderController {
    private final OrderService orderService;

    @PostMapping
    public ResponseEntity<OrderResponse> createOrder(@Valid @RequestBody CreateOrderRequest request) {
        OrderResponse order = orderService.create(request);
        return ResponseEntity.ok(order);
    }

    @GetMapping("/{id}")
    public ResponseEntity<OrderResponse> getOrder(@PathVariable UUID id) {
        return ResponseEntity.ok(orderService.findById(id));
    }
}

@Service
public class OrderService {
    private final OrderRepository orderRepository;
    private final PricingService pricingService;
    private final NotificationService notificationService;
    private final InventoryService inventoryService;

    public OrderResponse create(CreateOrderRequest request) {
        BigDecimal total = pricingService.calculate(request.getItems(), request.getCoupon());
        Order order = orderRepository.save(request.getItems(), total);
        inventoryService.deduct(request.getItems());
        notificationService.orderConfirmed(request.getEmail(), order);
        return OrderResponse.from(order);
    }
}

@Repository
public class OrderRepository {
    public Order save(List<OrderItem> items, BigDecimal total) { /* ... */ }
    public Order findById(UUID id) { /* ... */ }
    public List<Order> list(OrderFilter filter) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_body`, `method_declaration` — heuristic based on method name prefixes and annotations
- **Detection approach**: Categorize methods by name prefix/pattern (`get`/`find`/`list` = accessor, `validate`/`check` = validation, `save`/`update`/`delete` = persistence, `send`/`notify`/`email` = communication, `track`/`log` = observability, `calculate`/`compute` = business logic). Also consider annotation-based hints (`@GetMapping`/`@PostMapping` = HTTP, `@Transactional` = persistence). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    body: (class_body
      (method_declaration
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
