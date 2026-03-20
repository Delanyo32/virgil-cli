# God Objects -- C#

## Overview
God objects are classes that accumulate too many responsibilities — too many fields, too many methods, or too many lines — violating the Single Responsibility Principle. In C#, this commonly manifests as monolithic service classes, oversized ASP.NET controllers with 20+ actions, or "manager" classes that become dumping grounds for business logic.

## Why It's a Tech Debt Concern
God classes in C# become development bottlenecks as multiple feature branches touch the same file, causing constant merge conflicts. Testing requires extensive mocking of many injected dependencies, resulting in fragile test suites. The cognitive load of navigating a class with 30+ methods and 15+ dependencies makes code review superficial and increases the risk of unintended side effects during refactoring.

## Applicability
- **Relevance**: high (C#'s class-based OOP and DI patterns make god classes a common anti-pattern)
- **Languages covered**: `.cs`
- **Frameworks/libraries**: ASP.NET Core (god controllers with 20+ actions), Entity Framework (oversized DbContext), WPF/MAUI (god ViewModels), Blazor (god components)

---

## Pattern 1: Oversized Class

### Description
A class with 30+ methods, 15+ fields, or exceeding 500 lines. The class handles multiple unrelated concerns such as user management, authentication, email, file storage, and reporting all within one type.

### Bad Code (Anti-pattern)
```csharp
public class UserService
{
    private readonly IDbConnection _db;
    private readonly IPasswordHasher<User> _passwordHasher;
    private readonly IEmailSender _emailSender;
    private readonly IDistributedCache _cache;
    private readonly IAmazonS3 _s3Client;
    private readonly ILogger<UserService> _logger;
    private readonly IMetrics _metrics;
    private readonly IMessageBus _messageBus;
    private readonly IValidator<User> _validator;
    private readonly IAuditLog _auditLog;
    private readonly ISessionStore _sessionStore;
    private readonly IFeatureFlags _featureFlags;
    private readonly IRateLimiter _rateLimiter;
    private readonly ITwoFactorAuth _twoFactorAuth;
    private readonly ISearchClient _searchClient;
    private readonly IConfiguration _configuration;

    public async Task<User> CreateUserAsync(CreateUserRequest request) { /* ... */ }
    public async Task<User> UpdateUserAsync(Guid id, UpdateUserRequest request) { /* ... */ }
    public async Task DeleteUserAsync(Guid id) { /* ... */ }
    public async Task<User> FindUserAsync(Guid id) { /* ... */ }
    public async Task<PagedResult<User>> ListUsersAsync(UserFilter filter) { /* ... */ }
    public async Task<List<User>> SearchUsersAsync(string query) { /* ... */ }
    public bool ValidateEmail(string email) { /* ... */ }
    public bool ValidatePassword(string password) { /* ... */ }
    public bool ValidateUsername(string username) { /* ... */ }
    public string HashPassword(string password) { /* ... */ }
    public bool VerifyPassword(string plain, string hashed) { /* ... */ }
    public string GenerateToken(User user) { /* ... */ }
    public ClaimsPrincipal VerifyToken(string token) { /* ... */ }
    public string RefreshToken(string token) { /* ... */ }
    public async Task SendWelcomeEmailAsync(User user) { /* ... */ }
    public async Task SendResetEmailAsync(User user) { /* ... */ }
    public async Task SendVerificationEmailAsync(User user) { /* ... */ }
    public async Task<string> UploadAvatarAsync(User user, IFormFile file) { /* ... */ }
    public Image ResizeAvatar(IFormFile file, int size) { /* ... */ }
    public async Task CacheUserAsync(User user) { /* ... */ }
    public async Task InvalidateCacheAsync(Guid userId) { /* ... */ }
    public async Task LogActivityAsync(User user, string action) { /* ... */ }
    public async Task<bool> CheckPermissionAsync(User user, string resource) { /* ... */ }
    public void TrackMetric(string eventName, Dictionary<string, object> data) { /* ... */ }
    public async Task<bool> RateLimitCheckAsync(Guid userId, string action) { /* ... */ }
    public async Task<byte[]> ExportUserDataAsync(User user) { /* ... */ }
    public async Task ImportUsersAsync(IFormFile csvFile) { /* ... */ }
    public async Task<Report> GenerateReportAsync(ReportFilter filter) { /* ... */ }
    public async Task IndexUserAsync(User user) { /* ... */ }
    public async Task ReindexAllAsync() { /* ... */ }
    public async Task<TwoFactorSetup> SetupTwoFactorAsync(User user) { /* ... */ }
    public async Task<bool> VerifyTwoFactorAsync(User user, string code) { /* ... */ }
}
```

### Good Code (Fix)
```csharp
public class UserService
{
    private readonly IUserRepository _userRepository;

    public async Task<User> CreateAsync(CreateUserRequest request) { /* ... */ }
    public async Task<User> UpdateAsync(Guid id, UpdateUserRequest request) { /* ... */ }
    public async Task DeleteAsync(Guid id) { /* ... */ }
    public async Task<User> FindByIdAsync(Guid id) { /* ... */ }
    public async Task<PagedResult<User>> ListAsync(UserFilter filter) { /* ... */ }
}

public class AuthService
{
    private readonly IPasswordHasher<User> _passwordHasher;
    private readonly IJwtTokenProvider _tokenProvider;

    public string HashPassword(string password) { /* ... */ }
    public bool VerifyPassword(string plain, string hashed) { /* ... */ }
    public string GenerateToken(User user) { /* ... */ }
    public ClaimsPrincipal VerifyToken(string token) { /* ... */ }
}

public class UserEmailService
{
    private readonly IEmailSender _emailSender;

    public async Task SendWelcomeAsync(User user) { /* ... */ }
    public async Task SendResetAsync(User user) { /* ... */ }
    public async Task SendVerificationAsync(User user) { /* ... */ }
}

public class UserSearchService
{
    private readonly ISearchClient _searchClient;

    public async Task<List<User>> SearchAsync(string query) { /* ... */ }
    public async Task IndexAsync(User user) { /* ... */ }
    public async Task ReindexAllAsync() { /* ... */ }
}

public class AvatarService
{
    private readonly IAmazonS3 _s3Client;

    public async Task<string> UploadAsync(User user, IFormFile file) { /* ... */ }
    public Image Resize(IFormFile file, int size) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_declaration` (count `field_declaration` and `method_declaration` children in `declaration_list`)
- **Detection approach**: Count `field_declaration` nodes and `method_declaration` nodes within a class's `declaration_list`. Flag when methods exceed 20, fields exceed 15, or total lines exceed 500. Include `constructor_declaration` and `property_declaration` in the count.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    name: (identifier) @class_name
    body: (declaration_list
      (method_declaration
        name: (identifier) @method_name)))

  (class_declaration
    body: (declaration_list
      (field_declaration) @field))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`, `god_controller`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class handling HTTP request processing, input validation, business logic, data access, and notification — a clear SRP violation. In C#, this frequently appears as an ASP.NET controller that bypasses the service/repository layers and performs all operations directly, or a service class that mixes domain logic with infrastructure concerns.

### Bad Code (Anti-pattern)
```csharp
[ApiController]
[Route("api/[controller]")]
public class OrderController : ControllerBase
{
    private readonly IDbConnection _db;
    private readonly IEmailSender _emailSender;
    private readonly IDistributedCache _cache;
    private readonly ILogger<OrderController> _logger;

    [HttpPost]
    public async Task<IActionResult> CreateOrder([FromBody] CreateOrderRequest request)
    {
        // Validation
        if (request.Items == null || !request.Items.Any())
            return BadRequest("No items provided");
        foreach (var item in request.Items)
            if (item.Quantity <= 0)
                return BadRequest("Invalid quantity");

        // Business logic
        var subtotal = request.Items.Sum(i => i.Price * i.Quantity);
        var tax = subtotal * 0.08m;
        var discount = CalculateDiscount(request.Coupon);
        var total = subtotal + tax - discount;

        // Database access
        var orderId = Guid.NewGuid();
        await _db.ExecuteAsync("INSERT INTO Orders ...", new { orderId, total, tax });
        foreach (var item in request.Items)
        {
            await _db.ExecuteAsync("INSERT INTO OrderItems ...", new { orderId, item.Id });
            await _db.ExecuteAsync("UPDATE Inventory SET Stock = Stock - @Qty ...", new { item.Quantity });
        }

        // Email
        await SendConfirmationEmailAsync(request.Email, orderId, total);

        // Logging
        _logger.LogInformation("Order {OrderId} created", orderId);

        return Ok(new { OrderId = orderId, Total = total });
    }

    private decimal CalculateDiscount(string coupon) { /* ... */ }
    private async Task SendConfirmationEmailAsync(string email, Guid orderId, decimal total) { /* ... */ }
    [HttpGet("{id}")] public async Task<IActionResult> GetOrder(Guid id) { /* ... */ }
    [HttpPut("{id}")] public async Task<IActionResult> UpdateOrder(Guid id, [FromBody] UpdateOrderRequest req) { /* ... */ }
    [HttpDelete("{id}")] public async Task<IActionResult> CancelOrder(Guid id) { /* ... */ }
    [HttpPost("{id}/refund")] public async Task<IActionResult> RefundOrder(Guid id) { /* ... */ }
    [HttpGet] public async Task<IActionResult> ListOrders([FromQuery] OrderFilter filter) { /* ... */ }
    [HttpGet("export")] public async Task<IActionResult> ExportOrders([FromQuery] string format) { /* ... */ }
    [HttpGet("{id}/invoice")] public async Task<IActionResult> GenerateInvoice(Guid id) { /* ... */ }
    private void ValidateCoupon(string code) { /* ... */ }
    private decimal CalculateShipping(List<OrderItem> items, Address address) { /* ... */ }
    private async Task NotifyWarehouseAsync(Guid orderId) { /* ... */ }
    private async Task UpdateInventoryAsync(List<OrderItem> items) { /* ... */ }
}
```

### Good Code (Fix)
```csharp
[ApiController]
[Route("api/[controller]")]
public class OrderController : ControllerBase
{
    private readonly IOrderService _orderService;

    [HttpPost]
    public async Task<IActionResult> CreateOrder([FromBody] CreateOrderRequest request)
    {
        var order = await _orderService.CreateAsync(request);
        return Ok(order);
    }

    [HttpGet("{id}")]
    public async Task<IActionResult> GetOrder(Guid id)
    {
        var order = await _orderService.FindByIdAsync(id);
        return Ok(order);
    }
}

public class OrderService : IOrderService
{
    private readonly IOrderRepository _orderRepository;
    private readonly IPricingService _pricingService;
    private readonly INotificationService _notificationService;
    private readonly IInventoryService _inventoryService;

    public async Task<OrderResponse> CreateAsync(CreateOrderRequest request)
    {
        var total = _pricingService.Calculate(request.Items, request.Coupon);
        var order = await _orderRepository.SaveAsync(request.Items, total);
        await _inventoryService.DeductAsync(request.Items);
        await _notificationService.OrderConfirmedAsync(request.Email, order);
        return OrderResponse.From(order);
    }
}

public class OrderRepository : IOrderRepository
{
    private readonly IDbConnection _db;

    public async Task<Order> SaveAsync(List<OrderItem> items, decimal total) { /* ... */ }
    public async Task<Order> FindByIdAsync(Guid id) { /* ... */ }
    public async Task<List<Order>> ListAsync(OrderFilter filter) { /* ... */ }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `declaration_list`, `method_declaration` — heuristic based on method name prefixes and attributes
- **Detection approach**: Categorize methods by name prefix/pattern (`Get`/`Find`/`List` = accessor, `Validate`/`Check` = validation, `Save`/`Update`/`Delete` = persistence, `Send`/`Notify`/`Email` = communication, `Track`/`Log` = observability, `Calculate`/`Compute` = business logic). Also consider attribute-based hints (`[HttpGet]`/`[HttpPost]` = HTTP, `[Transactional]` = persistence). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_declaration
    body: (declaration_list
      (method_declaration
        name: (identifier) @method_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_class`, `god_controller`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
