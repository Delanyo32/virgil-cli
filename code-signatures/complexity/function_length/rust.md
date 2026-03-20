# Function Length -- Rust

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```rust
fn process_records(path: &Path, config: &Config) -> Result<Summary, AppError> {
    // Validate input
    if !path.exists() {
        return Err(AppError::NotFound(format!("File not found: {}", path.display())));
    }
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| AppError::InvalidInput("Missing file extension".into()))?;
    if extension != "csv" {
        return Err(AppError::InvalidInput(format!("Expected CSV, got .{}", extension)));
    }

    // Read and parse
    let content = std::fs::read_to_string(path)
        .map_err(|e| AppError::Io(format!("Failed to read {}: {}", path.display(), e)))?;
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in content.lines().enumerate().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 4 {
            errors.push(format!("Line {}: insufficient fields", i + 1));
            continue;
        }
        let amount: f64 = match fields[2].trim().parse() {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("Line {}: invalid amount: {}", i + 1, e));
                continue;
            }
        };
        let date = match NaiveDate::parse_from_str(fields[3].trim(), "%Y-%m-%d") {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("Line {}: invalid date: {}", i + 1, e));
                continue;
            }
        };
        records.push(Record {
            id: fields[0].trim().to_string(),
            name: fields[1].trim().to_string(),
            amount,
            date,
        });
    }

    if records.is_empty() {
        return Err(AppError::InvalidInput("No valid records found".into()));
    }

    // Compute aggregations
    let total: f64 = records.iter().map(|r| r.amount).sum();
    let average = total / records.len() as f64;
    let min_amount = records.iter().map(|r| r.amount).fold(f64::INFINITY, f64::min);
    let max_amount = records.iter().map(|r| r.amount).fold(f64::NEG_INFINITY, f64::max);
    let mut by_month: HashMap<String, Vec<&Record>> = HashMap::new();
    for record in &records {
        let key = record.date.format("%Y-%m").to_string();
        by_month.entry(key).or_default().push(record);
    }

    // Write output
    let output_path = config.output_dir.join("summary.json");
    let summary = Summary {
        record_count: records.len(),
        error_count: errors.len(),
        total,
        average,
        min_amount,
        max_amount,
        months: by_month.len(),
    };
    let json = serde_json::to_string_pretty(&summary)
        .map_err(|e| AppError::Serialization(e.to_string()))?;
    std::fs::write(&output_path, &json)
        .map_err(|e| AppError::Io(format!("Failed to write {}: {}", output_path.display(), e)))?;

    if !errors.is_empty() {
        let error_path = config.output_dir.join("errors.log");
        std::fs::write(&error_path, errors.join("\n"))
            .map_err(|e| AppError::Io(format!("Failed to write errors: {}", e)))?;
    }

    Ok(summary)
}
```

### Good Code (Fix)
```rust
fn validate_input(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::NotFound(format!("File not found: {}", path.display())));
    }
    let ext = path.extension().and_then(|e| e.to_str());
    if ext != Some("csv") {
        return Err(AppError::InvalidInput("Expected CSV file".into()));
    }
    Ok(())
}

fn parse_record(fields: &[&str], line_num: usize) -> Result<Record, String> {
    if fields.len() < 4 {
        return Err(format!("Line {}: insufficient fields", line_num));
    }
    let amount: f64 = fields[2].trim().parse()
        .map_err(|e| format!("Line {}: invalid amount: {}", line_num, e))?;
    let date = NaiveDate::parse_from_str(fields[3].trim(), "%Y-%m-%d")
        .map_err(|e| format!("Line {}: invalid date: {}", line_num, e))?;
    Ok(Record {
        id: fields[0].trim().to_string(),
        name: fields[1].trim().to_string(),
        amount,
        date,
    })
}

fn parse_csv(path: &Path) -> Result<(Vec<Record>, Vec<String>), AppError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AppError::Io(format!("Failed to read {}: {}", path.display(), e)))?;
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in content.lines().enumerate().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        match parse_record(&fields, i + 1) {
            Ok(r) => records.push(r),
            Err(e) => errors.push(e),
        }
    }
    Ok((records, errors))
}

fn compute_summary(records: &[Record], error_count: usize) -> Summary {
    let total: f64 = records.iter().map(|r| r.amount).sum();
    let months: HashSet<_> = records.iter()
        .map(|r| r.date.format("%Y-%m").to_string())
        .collect();
    Summary {
        record_count: records.len(),
        error_count,
        total,
        average: total / records.len() as f64,
        min_amount: records.iter().map(|r| r.amount).fold(f64::INFINITY, f64::min),
        max_amount: records.iter().map(|r| r.amount).fold(f64::NEG_INFINITY, f64::max),
        months: months.len(),
    }
}

fn process_records(path: &Path, config: &Config) -> Result<Summary, AppError> {
    validate_input(path)?;
    let (records, errors) = parse_csv(path)?;
    if records.is_empty() {
        return Err(AppError::InvalidInput("No valid records found".into()));
    }
    let summary = compute_summary(&records, errors.len());
    write_summary(&summary, &config.output_dir)?;
    if !errors.is_empty() {
        write_errors(&errors, &config.output_dir)?;
    }
    Ok(summary)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (function_item
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
A single Actix-web/Axum/Rocket handler function that contains all business logic inline -- request extraction, validation, database queries, response building -- instead of delegating to service functions.

### Bad Code (Anti-pattern)
```rust
async fn create_user(
    pool: web::Data<DbPool>,
    body: web::Json<CreateUserRequest>,
) -> Result<HttpResponse, AppError> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Ok(HttpResponse::BadRequest().json(json!({"error": "Invalid email"})));
    }
    if body.password.len() < 8 {
        return Ok(HttpResponse::BadRequest().json(json!({"error": "Password too short"})));
    }
    let name = body.name.trim().to_string();
    if name.len() < 2 {
        return Ok(HttpResponse::BadRequest().json(json!({"error": "Name too short"})));
    }
    let conn = pool.get()
        .map_err(|e| AppError::Internal(format!("DB pool error: {}", e)))?;
    let existing: Option<User> = users::table
        .filter(users::email.eq(&email))
        .first(&conn)
        .optional()
        .map_err(|e| AppError::Internal(format!("Query error: {}", e)))?;
    if existing.is_some() {
        return Ok(HttpResponse::Conflict().json(json!({"error": "Email taken"})));
    }
    let hashed = bcrypt::hash(&body.password, bcrypt::DEFAULT_COST)
        .map_err(|e| AppError::Internal(format!("Hash error: {}", e)))?;
    let new_user = NewUser {
        email: &email,
        password_hash: &hashed,
        name: &name,
        role: "user",
        verified: false,
    };
    let user: User = diesel::insert_into(users::table)
        .values(&new_user)
        .get_result(&conn)
        .map_err(|e| AppError::Internal(format!("Insert error: {}", e)))?;
    let token = encode_jwt(user.id, Duration::hours(24))?;
    send_verification_email(&user.email, &user.name, &token).await?;
    diesel::insert_into(audit_log::table)
        .values(&NewAuditEntry {
            action: "user_created",
            user_id: user.id,
            timestamp: Utc::now().naive_utc(),
        })
        .execute(&conn)
        .map_err(|e| AppError::Internal(format!("Audit error: {}", e)))?;
    Ok(HttpResponse::Created().json(json!({
        "id": user.id,
        "email": user.email,
        "name": user.name,
    })))
}
```

### Good Code (Fix)
```rust
async fn create_user(
    pool: web::Data<DbPool>,
    body: web::Json<CreateUserRequest>,
) -> Result<HttpResponse, AppError> {
    let input = validate_registration(&body)?;
    let conn = pool.get()
        .map_err(|e| AppError::Internal(format!("DB pool error: {}", e)))?;
    let user = user_service::register(&conn, input).await?;
    Ok(HttpResponse::Created().json(UserResponse::from(user)))
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40. Handler functions are identified the same way as regular functions -- the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (function_item
    name: (identifier) @func.name
    body: (block) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
