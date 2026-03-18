# Function Length -- Python

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .py, .pyi
- **Threshold**: 50 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 50-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```python
def process_csv_report(file_path, output_dir, config):
    """Process a CSV report file and generate summaries."""
    # Validate inputs
    if not os.path.exists(file_path):
        raise FileNotFoundError(f"Input file not found: {file_path}")
    if not file_path.endswith(".csv"):
        raise ValueError("Input must be a CSV file")
    os.makedirs(output_dir, exist_ok=True)

    # Read and parse CSV
    records = []
    with open(file_path, "r", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            if not row.get("id"):
                continue
            try:
                record = {
                    "id": int(row["id"]),
                    "name": row["name"].strip(),
                    "amount": float(row["amount"]),
                    "date": datetime.strptime(row["date"], "%Y-%m-%d"),
                    "category": row.get("category", "uncategorized").lower(),
                    "status": row.get("status", "pending"),
                }
            except (ValueError, KeyError) as e:
                logger.warning(f"Skipping malformed row {row.get('id')}: {e}")
                continue
            if record["amount"] < 0:
                logger.warning(f"Negative amount for record {record['id']}")
            records.append(record)

    if not records:
        raise ValueError("No valid records found in file")

    # Compute aggregations
    by_category = defaultdict(list)
    for r in records:
        by_category[r["category"]].append(r)

    summaries = {}
    for category, items in by_category.items():
        amounts = [i["amount"] for i in items]
        summaries[category] = {
            "count": len(items),
            "total": sum(amounts),
            "average": sum(amounts) / len(amounts),
            "min": min(amounts),
            "max": max(amounts),
            "pending": sum(1 for i in items if i["status"] == "pending"),
            "completed": sum(1 for i in items if i["status"] == "completed"),
        }

    # Generate output files
    summary_path = os.path.join(output_dir, "summary.json")
    with open(summary_path, "w") as f:
        json.dump(summaries, f, indent=2, default=str)

    detail_path = os.path.join(output_dir, "details.csv")
    with open(detail_path, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=["id", "name", "amount", "date", "category", "status"])
        writer.writeheader()
        for r in sorted(records, key=lambda x: x["date"]):
            writer.writerow({
                "id": r["id"],
                "name": r["name"],
                "amount": f"{r['amount']:.2f}",
                "date": r["date"].strftime("%Y-%m-%d"),
                "category": r["category"],
                "status": r["status"],
            })

    # Send notification
    if config.get("notify"):
        total_amount = sum(s["total"] for s in summaries.values())
        message = f"Report processed: {len(records)} records, ${total_amount:.2f} total"
        send_notification(config["notify"]["channel"], message)

    return {
        "records_processed": len(records),
        "categories": len(summaries),
        "output_dir": output_dir,
    }
```

### Good Code (Fix)
```python
def _validate_report_inputs(file_path, output_dir):
    if not os.path.exists(file_path):
        raise FileNotFoundError(f"Input file not found: {file_path}")
    if not file_path.endswith(".csv"):
        raise ValueError("Input must be a CSV file")
    os.makedirs(output_dir, exist_ok=True)


def _parse_csv_records(file_path):
    records = []
    with open(file_path, "r", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            record = _parse_row(row)
            if record is not None:
                records.append(record)
    if not records:
        raise ValueError("No valid records found in file")
    return records


def _parse_row(row):
    if not row.get("id"):
        return None
    try:
        return {
            "id": int(row["id"]),
            "name": row["name"].strip(),
            "amount": float(row["amount"]),
            "date": datetime.strptime(row["date"], "%Y-%m-%d"),
            "category": row.get("category", "uncategorized").lower(),
            "status": row.get("status", "pending"),
        }
    except (ValueError, KeyError) as e:
        logger.warning(f"Skipping malformed row {row.get('id')}: {e}")
        return None


def _compute_category_summaries(records):
    by_category = defaultdict(list)
    for r in records:
        by_category[r["category"]].append(r)

    return {
        category: _summarize_category(items)
        for category, items in by_category.items()
    }


def _summarize_category(items):
    amounts = [i["amount"] for i in items]
    return {
        "count": len(items),
        "total": sum(amounts),
        "average": sum(amounts) / len(amounts),
        "min": min(amounts),
        "max": max(amounts),
        "pending": sum(1 for i in items if i["status"] == "pending"),
        "completed": sum(1 for i in items if i["status"] == "completed"),
    }


def process_csv_report(file_path, output_dir, config):
    """Process a CSV report file and generate summaries."""
    _validate_report_inputs(file_path, output_dir)
    records = _parse_csv_records(file_path)
    summaries = _compute_category_summaries(records)
    _write_summary_json(summaries, output_dir)
    _write_detail_csv(records, output_dir)

    if config.get("notify"):
        total = sum(s["total"] for s in summaries.values())
        send_notification(config["notify"]["channel"],
                          f"Report processed: {len(records)} records, ${total:.2f} total")

    return {
        "records_processed": len(records),
        "categories": len(summaries),
        "output_dir": output_dir,
    }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`
- **Detection approach**: Count lines in the function body block. Python uses indentation-based blocks, so count lines from the first statement to the last statement in the function body. Flag when line count exceeds 50.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    name: (identifier) @func.name
    body: (block) @func.body)

  (decorated_definition
    definition: (function_definition
      name: (identifier) @func.name
      body: (block) @func.body))
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single Flask/Django/FastAPI view function that contains all business logic inline -- request parsing, validation, database queries, response formatting -- instead of delegating to service functions.

### Bad Code (Anti-pattern)
```python
@app.route("/api/users", methods=["POST"])
def create_user():
    data = request.get_json()
    if not data:
        return jsonify({"error": "Request body required"}), 400
    email = data.get("email", "").strip().lower()
    if not email or "@" not in email:
        return jsonify({"error": "Valid email required"}), 400
    password = data.get("password", "")
    if len(password) < 8:
        return jsonify({"error": "Password must be at least 8 characters"}), 400
    name = data.get("name", "").strip()
    if len(name) < 2:
        return jsonify({"error": "Name must be at least 2 characters"}), 400
    existing = User.query.filter_by(email=email).first()
    if existing:
        return jsonify({"error": "Email already registered"}), 409
    hashed = bcrypt.generate_password_hash(password).decode("utf-8")
    user = User(email=email, password=hashed, name=name, role="user",
                verified=False, created_at=datetime.utcnow())
    db.session.add(user)
    db.session.commit()
    token = create_access_token(identity=user.id, expires_delta=timedelta(hours=24))
    verification_url = f"{app.config['BASE_URL']}/verify?token={token}"
    msg = Message("Verify your account", recipients=[user.email])
    msg.html = render_template("verify_email.html", user=user, url=verification_url)
    mail.send(msg)
    audit = AuditLog(action="user_created", user_id=user.id,
                     ip_address=request.remote_addr, timestamp=datetime.utcnow())
    db.session.add(audit)
    db.session.commit()
    return jsonify({
        "id": user.id,
        "email": user.email,
        "name": user.name,
        "message": "Registration successful. Check your email to verify.",
    }), 201
```

### Good Code (Fix)
```python
@app.route("/api/users", methods=["POST"])
def create_user():
    data = request.get_json()
    errors = validate_registration(data)
    if errors:
        return jsonify({"errors": errors}), 400

    result = user_service.register(data, ip_address=request.remote_addr)
    return jsonify(result), 201
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `decorated_definition`
- **Detection approach**: Count lines in the function body block. Flag when line count exceeds 50. Handler functions decorated with `@app.route` or similar decorators are detected the same way; the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (decorated_definition
    (decorator
      (call
        function: (attribute) @decorator.name))
    definition: (function_definition
      name: (identifier) @func.name
      body: (block) @func.body))
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
