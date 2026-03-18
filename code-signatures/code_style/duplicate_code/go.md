# Duplicate Code -- Go

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .go
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters.

### Bad Code (Anti-pattern)
```go
func HandleCreateUser(w http.ResponseWriter, r *http.Request) {
	var payload UserPayload
	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		http.Error(w, "invalid request body", http.StatusBadRequest)
		return
	}
	if payload.Name == "" || payload.Email == "" {
		http.Error(w, "name and email are required", http.StatusBadRequest)
		return
	}
	normalized := strings.TrimSpace(strings.ToLower(payload.Name))
	record := User{Name: normalized, Email: payload.Email, CreatedAt: time.Now()}
	if err := db.Create(&record).Error; err != nil {
		http.Error(w, "failed to create user", http.StatusInternalServerError)
		return
	}
	log.Printf("Created user %d", record.ID)
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(record)
}

func HandleCreateVendor(w http.ResponseWriter, r *http.Request) {
	var payload VendorPayload
	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		http.Error(w, "invalid request body", http.StatusBadRequest)
		return
	}
	if payload.Name == "" || payload.Email == "" {
		http.Error(w, "name and email are required", http.StatusBadRequest)
		return
	}
	normalized := strings.TrimSpace(strings.ToLower(payload.Name))
	record := Vendor{Name: normalized, Email: payload.Email, CreatedAt: time.Now()}
	if err := db.Create(&record).Error; err != nil {
		http.Error(w, "failed to create vendor", http.StatusInternalServerError)
		return
	}
	log.Printf("Created vendor %d", record.ID)
	w.WriteHeader(http.StatusCreated)
	json.NewEncoder(w).Encode(record)
}
```

### Good Code (Fix)
```go
type Entity interface {
	SetName(string)
	SetEmail(string)
	SetCreatedAt(time.Time)
	EntityType() string
}

func HandleCreateEntity[T Entity](db *gorm.DB) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		var payload struct {
			Name  string `json:"name"`
			Email string `json:"email"`
		}
		if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
			http.Error(w, "invalid request body", http.StatusBadRequest)
			return
		}
		if payload.Name == "" || payload.Email == "" {
			http.Error(w, "name and email are required", http.StatusBadRequest)
			return
		}
		var record T
		record.SetName(strings.TrimSpace(strings.ToLower(payload.Name)))
		record.SetEmail(payload.Email)
		record.SetCreatedAt(time.Now())
		if err := db.Create(&record).Error; err != nil {
			http.Error(w, fmt.Sprintf("failed to create %s", record.EntityType()), http.StatusInternalServerError)
			return
		}
		log.Printf("Created %s", record.EntityType())
		w.WriteHeader(http.StatusCreated)
		json.NewEncoder(w).Encode(record)
	}
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `method_declaration`, `block`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_declaration
  name: (identifier) @func_name
  body: (block) @func_body)

(method_declaration
  name: (field_identifier) @func_name
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
The same sequence of 5+ statements repeated within a function or across methods on the same receiver, often due to copy-paste during development. Common in duplicated HTTP handler logic and repeated error handling blocks.

### Bad Code (Anti-pattern)
```go
func (s *Service) ProcessBatch(items []Item) error {
	for _, item := range items {
		if item.Type == "typeA" {
			conn, err := s.pool.Get()
			if err != nil {
				log.Printf("pool error: %v", err)
				return fmt.Errorf("pool error: %w", err)
			}
			defer conn.Close()
			result, err := conn.Execute("INSERT INTO type_a VALUES (?, ?, ?)", item.ID, item.Name, item.Value)
			if err != nil {
				log.Printf("insert error for %s: %v", item.ID, err)
				return fmt.Errorf("insert error: %w", err)
			}
			log.Printf("Inserted type_a: %d rows affected", result.RowsAffected)
		} else if item.Type == "typeB" {
			conn, err := s.pool.Get()
			if err != nil {
				log.Printf("pool error: %v", err)
				return fmt.Errorf("pool error: %w", err)
			}
			defer conn.Close()
			result, err := conn.Execute("INSERT INTO type_b VALUES (?, ?, ?)", item.ID, item.Name, item.Value)
			if err != nil {
				log.Printf("insert error for %s: %v", item.ID, err)
				return fmt.Errorf("insert error: %w", err)
			}
			log.Printf("Inserted type_b: %d rows affected", result.RowsAffected)
		}
	}
	return nil
}
```

### Good Code (Fix)
```go
func (s *Service) insertItem(table string, item Item) error {
	conn, err := s.pool.Get()
	if err != nil {
		log.Printf("pool error: %v", err)
		return fmt.Errorf("pool error: %w", err)
	}
	defer conn.Close()
	result, err := conn.Execute(fmt.Sprintf("INSERT INTO %s VALUES (?, ?, ?)", table), item.ID, item.Name, item.Value)
	if err != nil {
		log.Printf("insert error for %s: %v", item.ID, err)
		return fmt.Errorf("insert error: %w", err)
	}
	log.Printf("Inserted %s: %d rows affected", table, result.RowsAffected)
	return nil
}

func (s *Service) ProcessBatch(items []Item) error {
	tableMap := map[string]string{"typeA": "type_a", "typeB": "type_b"}
	for _, item := range items {
		table, ok := tableMap[item.Type]
		if !ok {
			continue
		}
		if err := s.insertItem(table, item); err != nil {
			return err
		}
	}
	return nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `block`, `short_var_declaration`, `if_statement`, `expression_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences. Go's verbose error handling pattern makes duplication particularly common.
- **S-expression query sketch**:
```scheme
(block
  (_) @stmt)

(if_statement
  consequence: (block
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
