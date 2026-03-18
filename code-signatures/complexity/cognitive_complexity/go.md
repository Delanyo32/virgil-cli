# Cognitive Complexity -- Go

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, continue, break, goto), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.go`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, select/switch, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```go
func processRecords(records []Record, config Config) ([]Result, error) {
	var results []Result
	for _, record := range records {
		if record.IsActive {
			switch record.Category {
			case "priority":
				for _, field := range record.Fields {
					if contains(config.RequiredFields, field.Name) {
						if field.Value == "" {
							continue
						}
						val, err := transform(field)
						if err != nil {
							if config.Strict {
								return nil, err
							}
							break
						}
						results = append(results, val)
					}
				}
			case "standard":
				if record.Fallback != nil {
					val, err := defaultTransform(record.Fallback)
					if err != nil {
						continue
					}
					results = append(results, val)
				}
			}
		}
	}
	return results, nil
}
```

### Good Code (Fix)
```go
func processPriorityFields(fields []Field, config Config) ([]Result, error) {
	var results []Result
	for _, field := range fields {
		if !contains(config.RequiredFields, field.Name) {
			continue
		}
		if field.Value == "" {
			continue
		}
		val, err := transform(field)
		if err != nil {
			if config.Strict {
				return nil, err
			}
			break
		}
		results = append(results, val)
	}
	return results, nil
}

func processSingleRecord(record Record, config Config) ([]Result, error) {
	if !record.IsActive {
		return nil, nil
	}
	switch record.Category {
	case "priority":
		return processPriorityFields(record.Fields, config)
	case "standard":
		if record.Fallback == nil {
			return nil, nil
		}
		val, err := defaultTransform(record.Fallback)
		if err != nil {
			return nil, nil
		}
		return []Result{val}, nil
	default:
		return nil, nil
	}
}

func processRecords(records []Record, config Config) ([]Result, error) {
	var results []Result
	for _, record := range records {
		partial, err := processSingleRecord(record, config)
		if err != nil {
			return nil, err
		}
		results = append(results, partial...)
	}
	return results, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `expression_switch_statement`, `type_switch_statement`, `select_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, `goto`, and early `return` statements add 1 each for flow disruption. `else` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_declaration body: (block) @fn_body) @fn
(method_declaration body: (block) @fn_body) @fn
(func_literal body: (block) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(expression_switch_statement) @nesting
(type_switch_statement) @nesting
(select_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(goto_statement) @flow_break

;; Else clauses break linear flow
(if_statement
  alternative: (_) @else_branch)
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and `if err != nil` checks at every step, creating a zigzag pattern that fragments the readable logic flow. This is Go's most common source of high cognitive complexity.

### Bad Code (Anti-pattern)
```go
func syncUserData(userID string) (*SyncResult, error) {
	user, err := fetchUser(userID)
	if err != nil {
		return nil, fmt.Errorf("fetch user: %w", err)
	}

	profile, err := fetchProfile(user.ProfileID)
	if err != nil {
		return nil, fmt.Errorf("fetch profile: %w", err)
	}

	preferences, err := loadPreferences(user.ID)
	if err != nil {
		preferences = defaultPreferences()
	}

	merged, err := mergeData(user, profile, preferences)
	if err != nil {
		return nil, fmt.Errorf("merge data: %w", err)
	}

	err = saveToCache(merged)
	if err != nil {
		log.Printf("cache save failed: %v", err)
	}

	err = notifyServices(merged)
	if err != nil {
		return nil, fmt.Errorf("notify: %w", err)
	}

	return &SyncResult{Data: merged}, nil
}
```

### Good Code (Fix)
```go
type syncPipeline struct {
	userID string
	user   *User
	merged *MergedData
}

func (p *syncPipeline) loadData() error {
	user, err := fetchUser(p.userID)
	if err != nil {
		return fmt.Errorf("fetch user: %w", err)
	}
	p.user = user

	profile, err := fetchProfile(user.ProfileID)
	if err != nil {
		return fmt.Errorf("fetch profile: %w", err)
	}

	preferences, err := loadPreferences(user.ID)
	if err != nil {
		preferences = defaultPreferences()
	}

	p.merged, err = mergeData(user, profile, preferences)
	if err != nil {
		return fmt.Errorf("merge data: %w", err)
	}
	return nil
}

func (p *syncPipeline) publish() error {
	if err := saveToCache(p.merged); err != nil {
		log.Printf("cache save failed: %v", err)
	}
	return notifyServices(p.merged)
}

func syncUserData(userID string) (*SyncResult, error) {
	p := &syncPipeline{userID: userID}
	if err := p.loadData(); err != nil {
		return nil, err
	}
	if err := p.publish(); err != nil {
		return nil, fmt.Errorf("notify: %w", err)
	}
	return &SyncResult{Data: p.merged}, nil
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `short_var_declaration`, `assignment_statement`
- **Detection approach**: Detect the idiomatic Go error-check pattern: a `short_var_declaration` or `assignment_statement` followed immediately by an `if_statement` that tests `err != nil`. Count these pairs within a single function body. If 4 or more appear as siblings, flag as interleaved error handling. Each error check adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect if err != nil pattern
(if_statement
  condition: (binary_expression
    left: (identifier) @err_var
    operator: "!="
    right: (nil))
  (#eq? @err_var "err")) @err_check

;; Count multiple error checks in a function
(function_declaration
  body: (block
    (if_statement
      condition: (binary_expression
        left: (identifier) @e1
        operator: "!="
        right: (nil))
      (#eq? @e1 "err")) @check1
    (if_statement
      condition: (binary_expression
        left: (identifier) @e2
        operator: "!="
        right: (nil))
      (#eq? @e2 "err")) @check2
    (if_statement
      condition: (binary_expression
        left: (identifier) @e3
        operator: "!="
        right: (nil))
      (#eq? @e3 "err")) @check3
    (if_statement
      condition: (binary_expression
        left: (identifier) @e4
        operator: "!="
        right: (nil))
      (#eq? @e4 "err")) @check4))
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
