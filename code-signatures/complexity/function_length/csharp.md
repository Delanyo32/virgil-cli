# Function Length -- C#

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .cs
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```csharp
public async Task<ImportResult> ImportCsvData(string filePath, ImportOptions options)
{
    // Validate inputs
    if (string.IsNullOrWhiteSpace(filePath))
        throw new ArgumentException("File path is required", nameof(filePath));
    if (!File.Exists(filePath))
        throw new FileNotFoundException($"File not found: {filePath}");
    if (Path.GetExtension(filePath).ToLower() != ".csv")
        throw new ArgumentException("File must be a CSV");
    if (options == null)
        throw new ArgumentNullException(nameof(options));

    // Read and parse
    var lines = await File.ReadAllLinesAsync(filePath);
    if (lines.Length < 2)
        throw new InvalidDataException("CSV must have a header and at least one data row");

    var headers = lines[0].Split(',').Select(h => h.Trim().ToLower()).ToArray();
    var requiredHeaders = new[] { "id", "name", "amount", "date" };
    var missing = requiredHeaders.Except(headers);
    if (missing.Any())
        throw new InvalidDataException($"Missing required headers: {string.Join(", ", missing)}");

    var records = new List<DataRecord>();
    var errors = new List<string>();
    for (int i = 1; i < lines.Length; i++)
    {
        var fields = lines[i].Split(',');
        if (fields.Length != headers.Length)
        {
            errors.Add($"Row {i}: field count mismatch");
            continue;
        }
        if (!int.TryParse(fields[Array.IndexOf(headers, "id")].Trim(), out var id))
        {
            errors.Add($"Row {i}: invalid ID");
            continue;
        }
        if (!decimal.TryParse(fields[Array.IndexOf(headers, "amount")].Trim(), out var amount))
        {
            errors.Add($"Row {i}: invalid amount");
            continue;
        }
        if (!DateTime.TryParseExact(fields[Array.IndexOf(headers, "date")].Trim(), "yyyy-MM-dd",
            CultureInfo.InvariantCulture, DateTimeStyles.None, out var date))
        {
            errors.Add($"Row {i}: invalid date");
            continue;
        }
        records.Add(new DataRecord
        {
            Id = id,
            Name = fields[Array.IndexOf(headers, "name")].Trim(),
            Amount = amount,
            Date = date,
        });
    }

    if (!records.Any())
        throw new InvalidDataException("No valid records found");

    // Compute summary
    var total = records.Sum(r => r.Amount);
    var average = records.Average(r => r.Amount);
    var byMonth = records.GroupBy(r => r.Date.ToString("yyyy-MM"))
        .ToDictionary(g => g.Key, g => g.Sum(r => r.Amount));

    // Persist
    if (options.SaveToDatabase)
    {
        using var transaction = await _dbContext.Database.BeginTransactionAsync();
        try
        {
            _dbContext.DataRecords.AddRange(records);
            await _dbContext.SaveChangesAsync();
            await transaction.CommitAsync();
        }
        catch (Exception ex)
        {
            await transaction.RollbackAsync();
            _logger.LogError(ex, "Failed to persist records");
            throw;
        }
    }

    // Write output
    var outputPath = Path.Combine(options.OutputDir, "summary.json");
    var summary = new ImportResult
    {
        RecordCount = records.Count,
        ErrorCount = errors.Count,
        Total = total,
        Average = average,
        MonthlyBreakdown = byMonth,
    };
    var json = JsonSerializer.Serialize(summary, new JsonSerializerOptions { WriteIndented = true });
    await File.WriteAllTextAsync(outputPath, json);

    return summary;
}
```

### Good Code (Fix)
```csharp
public async Task<ImportResult> ImportCsvData(string filePath, ImportOptions options)
{
    ValidateInputs(filePath, options);

    var (headers, dataLines) = await ReadCsvFile(filePath);
    ValidateHeaders(headers);

    var (records, errors) = ParseRecords(headers, dataLines);
    if (!records.Any())
        throw new InvalidDataException("No valid records found");

    var summary = ComputeSummary(records, errors);

    if (options.SaveToDatabase)
        await PersistRecords(records);

    await WriteSummary(summary, options.OutputDir);
    return summary;
}

private void ValidateInputs(string filePath, ImportOptions options)
{
    if (string.IsNullOrWhiteSpace(filePath))
        throw new ArgumentException("File path is required", nameof(filePath));
    if (!File.Exists(filePath))
        throw new FileNotFoundException($"File not found: {filePath}");
    if (Path.GetExtension(filePath).ToLower() != ".csv")
        throw new ArgumentException("File must be a CSV");
    if (options == null)
        throw new ArgumentNullException(nameof(options));
}

private (List<DataRecord> Records, List<string> Errors) ParseRecords(string[] headers, string[] lines)
{
    var records = new List<DataRecord>();
    var errors = new List<string>();
    for (int i = 0; i < lines.Length; i++)
    {
        var result = ParseRow(headers, lines[i], i + 1);
        if (result.HasValue)
            records.Add(result.Value);
        else
            errors.Add(result.Error);
    }
    return (records, errors);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `constructor_declaration`, `local_function_statement`
- **Detection approach**: Count lines between method body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    name: (identifier) @func.name
    body: (block) @func.body)

  (constructor_declaration
    name: (identifier) @func.name
    body: (block) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single ASP.NET controller action method that contains all business logic inline -- model validation, database access, business rules, response formatting -- instead of delegating to service classes.

### Bad Code (Anti-pattern)
```csharp
[HttpPost("api/users")]
public async Task<IActionResult> CreateUser([FromBody] Dictionary<string, string> body)
{
    if (!body.TryGetValue("email", out var email) || string.IsNullOrWhiteSpace(email) || !email.Contains("@"))
        return BadRequest(new { error = "Invalid email" });
    email = email.Trim().ToLower();
    if (!body.TryGetValue("password", out var password) || password.Length < 8)
        return BadRequest(new { error = "Password must be at least 8 characters" });
    if (!body.TryGetValue("name", out var name) || name.Trim().Length < 2)
        return BadRequest(new { error = "Name must be at least 2 characters" });
    name = name.Trim();
    var existing = await _dbContext.Users.FirstOrDefaultAsync(u => u.Email == email);
    if (existing != null)
        return Conflict(new { error = "Email already registered" });
    var hashedPassword = BCrypt.Net.BCrypt.HashPassword(password);
    var user = new User
    {
        Email = email,
        PasswordHash = hashedPassword,
        Name = name,
        Role = "User",
        IsVerified = false,
        CreatedAt = DateTime.UtcNow,
    };
    _dbContext.Users.Add(user);
    await _dbContext.SaveChangesAsync();
    var token = _jwtService.GenerateToken(user.Id, TimeSpan.FromHours(24));
    var verificationUrl = $"{_config["BaseUrl"]}/verify?token={token}";
    var mailMessage = new MailMessage
    {
        From = new MailAddress(_config["EmailFrom"]),
        Subject = "Verify your account",
        Body = $"<p>Hi {user.Name},</p><p><a href='{verificationUrl}'>Verify</a></p>",
        IsBodyHtml = true,
    };
    mailMessage.To.Add(user.Email);
    await _smtpClient.SendMailAsync(mailMessage);
    _dbContext.AuditLogs.Add(new AuditLog
    {
        Action = "UserCreated",
        UserId = user.Id,
        IpAddress = HttpContext.Connection.RemoteIpAddress?.ToString(),
        Timestamp = DateTime.UtcNow,
    });
    await _dbContext.SaveChangesAsync();
    return CreatedAtAction(nameof(GetUser), new { id = user.Id }, new
    {
        id = user.Id,
        email = user.Email,
        name = user.Name,
        message = "Registration successful. Check your email.",
    });
}
```

### Good Code (Fix)
```csharp
[HttpPost("api/users")]
public async Task<IActionResult> CreateUser([FromBody] CreateUserRequest request)
{
    if (!ModelState.IsValid)
        return BadRequest(ModelState);

    var result = await _userService.RegisterAsync(request, HttpContext.Connection.RemoteIpAddress?.ToString());
    return CreatedAtAction(nameof(GetUser), new { id = result.Id }, result);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`
- **Detection approach**: Count lines between method body opening and closing braces. Flag when line count exceeds 40. Methods with `[HttpPost]`, `[HttpGet]`, etc. attributes are detected the same way; the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    (attribute_list
      (attribute
        name: (identifier) @attr.name))
    name: (identifier) @func.name
    body: (block) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
