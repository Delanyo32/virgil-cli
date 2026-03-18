# Function Length -- Go

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```go
func SyncInventory(ctx context.Context, db *sql.DB, warehouseID string, items []InventoryUpdate) (*SyncResult, error) {
	// Validate inputs
	if warehouseID == "" {
		return nil, fmt.Errorf("warehouse ID is required")
	}
	if len(items) == 0 {
		return nil, fmt.Errorf("at least one item is required")
	}
	for i, item := range items {
		if item.SKU == "" {
			return nil, fmt.Errorf("item %d: SKU is required", i)
		}
		if item.Quantity < 0 {
			return nil, fmt.Errorf("item %d: quantity cannot be negative", i)
		}
	}

	// Check warehouse exists
	var warehouseName string
	err := db.QueryRowContext(ctx, "SELECT name FROM warehouses WHERE id = $1", warehouseID).Scan(&warehouseName)
	if err == sql.ErrNoRows {
		return nil, fmt.Errorf("warehouse %s not found", warehouseID)
	}
	if err != nil {
		return nil, fmt.Errorf("querying warehouse: %w", err)
	}

	// Begin transaction
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("starting transaction: %w", err)
	}
	defer tx.Rollback()

	var updated, created, failed int
	var errors []string
	for _, item := range items {
		var currentQty int
		err := tx.QueryRowContext(ctx, "SELECT quantity FROM inventory WHERE warehouse_id = $1 AND sku = $2",
			warehouseID, item.SKU).Scan(&currentQty)
		if err == sql.ErrNoRows {
			_, err = tx.ExecContext(ctx,
				"INSERT INTO inventory (warehouse_id, sku, quantity, updated_at) VALUES ($1, $2, $3, $4)",
				warehouseID, item.SKU, item.Quantity, time.Now())
			if err != nil {
				errors = append(errors, fmt.Sprintf("SKU %s: insert failed: %v", item.SKU, err))
				failed++
				continue
			}
			created++
		} else if err != nil {
			errors = append(errors, fmt.Sprintf("SKU %s: query failed: %v", item.SKU, err))
			failed++
			continue
		} else {
			newQty := currentQty + item.Quantity
			if newQty < 0 {
				errors = append(errors, fmt.Sprintf("SKU %s: would result in negative inventory", item.SKU))
				failed++
				continue
			}
			_, err = tx.ExecContext(ctx,
				"UPDATE inventory SET quantity = $1, updated_at = $2 WHERE warehouse_id = $3 AND sku = $4",
				newQty, time.Now(), warehouseID, item.SKU)
			if err != nil {
				errors = append(errors, fmt.Sprintf("SKU %s: update failed: %v", item.SKU, err))
				failed++
				continue
			}
			updated++
		}
	}

	// Record audit entry
	_, err = tx.ExecContext(ctx,
		"INSERT INTO audit_log (warehouse_id, action, details, timestamp) VALUES ($1, $2, $3, $4)",
		warehouseID, "inventory_sync",
		fmt.Sprintf("updated=%d created=%d failed=%d", updated, created, failed),
		time.Now())
	if err != nil {
		return nil, fmt.Errorf("recording audit log: %w", err)
	}

	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("committing transaction: %w", err)
	}

	return &SyncResult{
		Warehouse: warehouseName,
		Updated:   updated,
		Created:   created,
		Failed:    failed,
		Errors:    errors,
	}, nil
}
```

### Good Code (Fix)
```go
func validateSyncInput(warehouseID string, items []InventoryUpdate) error {
	if warehouseID == "" {
		return fmt.Errorf("warehouse ID is required")
	}
	if len(items) == 0 {
		return fmt.Errorf("at least one item is required")
	}
	for i, item := range items {
		if item.SKU == "" {
			return fmt.Errorf("item %d: SKU is required", i)
		}
		if item.Quantity < 0 {
			return fmt.Errorf("item %d: quantity cannot be negative", i)
		}
	}
	return nil
}

func upsertInventoryItem(ctx context.Context, tx *sql.Tx, warehouseID string, item InventoryUpdate) error {
	var currentQty int
	err := tx.QueryRowContext(ctx,
		"SELECT quantity FROM inventory WHERE warehouse_id = $1 AND sku = $2",
		warehouseID, item.SKU).Scan(&currentQty)

	if err == sql.ErrNoRows {
		_, err = tx.ExecContext(ctx,
			"INSERT INTO inventory (warehouse_id, sku, quantity, updated_at) VALUES ($1, $2, $3, $4)",
			warehouseID, item.SKU, item.Quantity, time.Now())
		return err
	}
	if err != nil {
		return err
	}

	newQty := currentQty + item.Quantity
	if newQty < 0 {
		return fmt.Errorf("would result in negative inventory")
	}
	_, err = tx.ExecContext(ctx,
		"UPDATE inventory SET quantity = $1, updated_at = $2 WHERE warehouse_id = $3 AND sku = $4",
		newQty, time.Now(), warehouseID, item.SKU)
	return err
}

func SyncInventory(ctx context.Context, db *sql.DB, warehouseID string, items []InventoryUpdate) (*SyncResult, error) {
	if err := validateSyncInput(warehouseID, items); err != nil {
		return nil, err
	}

	warehouseName, err := lookupWarehouse(ctx, db, warehouseID)
	if err != nil {
		return nil, err
	}

	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return nil, fmt.Errorf("starting transaction: %w", err)
	}
	defer tx.Rollback()

	result := processSyncItems(ctx, tx, warehouseID, items)
	if err := recordAudit(ctx, tx, warehouseID, result); err != nil {
		return nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, fmt.Errorf("committing transaction: %w", err)
	}

	result.Warehouse = warehouseName
	return result, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (function_declaration
    name: (identifier) @func.name
    body: (block) @func.body)

  (method_declaration
    name: (field_identifier) @func.name
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
A single `http.HandlerFunc` or Gin/Echo/Chi handler that contains all business logic inline -- request decoding, validation, database access, response encoding -- instead of delegating to service functions.

### Bad Code (Anti-pattern)
```go
func (s *Server) HandleCreateUser(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req CreateUserRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "Invalid JSON", http.StatusBadRequest)
		return
	}
	req.Email = strings.TrimSpace(strings.ToLower(req.Email))
	if req.Email == "" || !strings.Contains(req.Email, "@") {
		http.Error(w, "Invalid email", http.StatusBadRequest)
		return
	}
	if len(req.Password) < 8 {
		http.Error(w, "Password too short", http.StatusBadRequest)
		return
	}
	req.Name = strings.TrimSpace(req.Name)
	if len(req.Name) < 2 {
		http.Error(w, "Name too short", http.StatusBadRequest)
		return
	}
	var exists bool
	err := s.db.QueryRowContext(r.Context(), "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)", req.Email).Scan(&exists)
	if err != nil {
		log.Printf("DB error: %v", err)
		http.Error(w, "Internal error", http.StatusInternalServerError)
		return
	}
	if exists {
		http.Error(w, "Email already registered", http.StatusConflict)
		return
	}
	hashed, err := bcrypt.GenerateFromPassword([]byte(req.Password), bcrypt.DefaultCost)
	if err != nil {
		log.Printf("Hash error: %v", err)
		http.Error(w, "Internal error", http.StatusInternalServerError)
		return
	}
	var userID int
	err = s.db.QueryRowContext(r.Context(),
		"INSERT INTO users (email, password_hash, name, role, created_at) VALUES ($1, $2, $3, $4, $5) RETURNING id",
		req.Email, string(hashed), req.Name, "user", time.Now()).Scan(&userID)
	if err != nil {
		log.Printf("Insert error: %v", err)
		http.Error(w, "Internal error", http.StatusInternalServerError)
		return
	}
	token, err := s.generateToken(userID)
	if err != nil {
		log.Printf("Token error: %v", err)
		http.Error(w, "Internal error", http.StatusInternalServerError)
		return
	}
	go s.sendVerificationEmail(req.Email, req.Name, token)
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(map[string]interface{}{
		"id":    userID,
		"email": req.Email,
		"name":  req.Name,
	})
}
```

### Good Code (Fix)
```go
func (s *Server) HandleCreateUser(w http.ResponseWriter, r *http.Request) {
	var req CreateUserRequest
	if err := decodeJSON(r, &req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid JSON")
		return
	}

	if err := req.Validate(); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}

	user, err := s.userService.Register(r.Context(), req)
	if err != nil {
		handleServiceError(w, err)
		return
	}

	respondJSON(w, http.StatusCreated, UserResponse{
		ID: user.ID, Email: user.Email, Name: user.Name,
	})
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40. Handler functions are identified the same way as regular functions -- the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    name: (field_identifier) @func.name
    body: (block) @func.body)

  (function_declaration
    name: (identifier) @func.name
    body: (block) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
