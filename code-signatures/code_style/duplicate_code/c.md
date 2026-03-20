# Duplicate Code -- C

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .c, .h
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more functions with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared function with parameters. Common when duplicate utility functions appear across translation units.

### Bad Code (Anti-pattern)
```c
int process_user_record(const char *name, const char *email, sqlite3 *db) {
    if (name == NULL || email == NULL) {
        fprintf(stderr, "Error: name and email are required\n");
        return -1;
    }
    char normalized[256];
    strncpy(normalized, name, sizeof(normalized) - 1);
    normalized[sizeof(normalized) - 1] = '\0';
    for (int i = 0; normalized[i]; i++) {
        normalized[i] = tolower(normalized[i]);
    }
    sqlite3_stmt *stmt;
    const char *sql = "INSERT INTO users (name, email) VALUES (?, ?)";
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, NULL) != SQLITE_OK) {
        fprintf(stderr, "Failed to prepare statement: %s\n", sqlite3_errmsg(db));
        return -1;
    }
    sqlite3_bind_text(stmt, 1, normalized, -1, SQLITE_STATIC);
    sqlite3_bind_text(stmt, 2, email, -1, SQLITE_STATIC);
    int rc = sqlite3_step(stmt);
    sqlite3_finalize(stmt);
    return rc == SQLITE_DONE ? 0 : -1;
}

int process_vendor_record(const char *name, const char *email, sqlite3 *db) {
    if (name == NULL || email == NULL) {
        fprintf(stderr, "Error: name and email are required\n");
        return -1;
    }
    char normalized[256];
    strncpy(normalized, name, sizeof(normalized) - 1);
    normalized[sizeof(normalized) - 1] = '\0';
    for (int i = 0; normalized[i]; i++) {
        normalized[i] = tolower(normalized[i]);
    }
    sqlite3_stmt *stmt;
    const char *sql = "INSERT INTO vendors (name, email) VALUES (?, ?)";
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, NULL) != SQLITE_OK) {
        fprintf(stderr, "Failed to prepare statement: %s\n", sqlite3_errmsg(db));
        return -1;
    }
    sqlite3_bind_text(stmt, 1, normalized, -1, SQLITE_STATIC);
    sqlite3_bind_text(stmt, 2, email, -1, SQLITE_STATIC);
    int rc = sqlite3_step(stmt);
    sqlite3_finalize(stmt);
    return rc == SQLITE_DONE ? 0 : -1;
}
```

### Good Code (Fix)
```c
int insert_entity(const char *table, const char *name, const char *email, sqlite3 *db) {
    if (name == NULL || email == NULL) {
        fprintf(stderr, "Error: name and email are required\n");
        return -1;
    }
    char normalized[256];
    strncpy(normalized, name, sizeof(normalized) - 1);
    normalized[sizeof(normalized) - 1] = '\0';
    for (int i = 0; normalized[i]; i++) {
        normalized[i] = tolower(normalized[i]);
    }
    char sql[512];
    snprintf(sql, sizeof(sql), "INSERT INTO %s (name, email) VALUES (?, ?)", table);
    sqlite3_stmt *stmt;
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, NULL) != SQLITE_OK) {
        fprintf(stderr, "Failed to prepare statement: %s\n", sqlite3_errmsg(db));
        return -1;
    }
    sqlite3_bind_text(stmt, 1, normalized, -1, SQLITE_STATIC);
    sqlite3_bind_text(stmt, 2, email, -1, SQLITE_STATIC);
    int rc = sqlite3_step(stmt);
    sqlite3_finalize(stmt);
    return rc == SQLITE_DONE ? 0 : -1;
}

int process_user_record(const char *name, const char *email, sqlite3 *db) {
    return insert_entity("users", name, email, db);
}

int process_vendor_record(const char *name, const char *email, sqlite3 *db) {
    return insert_entity("vendors", name, email, db);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `compound_statement`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func_name)
  body: (compound_statement) @func_body)
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `cloned_function_bodies`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Repeated Logic Blocks Within a Function

### Description
The same sequence of 5+ statements repeated within a function or across functions in the same translation unit, often due to copy-paste during development. Common in duplicate utility functions for buffer manipulation, error-checked I/O, and resource lifecycle management.

### Bad Code (Anti-pattern)
```c
int process_batch(const char **names, const char **emails, int count, sqlite3 *db) {
    for (int i = 0; i < count; i++) {
        if (names[i] == NULL || emails[i] == NULL) {
            fprintf(stderr, "Skipping record %d: missing fields\n", i);
            continue;
        }

        /* Process as user */
        sqlite3_stmt *user_stmt;
        if (sqlite3_prepare_v2(db, "INSERT INTO users (name, email) VALUES (?, ?)",
                               -1, &user_stmt, NULL) != SQLITE_OK) {
            fprintf(stderr, "Prepare failed: %s\n", sqlite3_errmsg(db));
            return -1;
        }
        sqlite3_bind_text(user_stmt, 1, names[i], -1, SQLITE_STATIC);
        sqlite3_bind_text(user_stmt, 2, emails[i], -1, SQLITE_STATIC);
        if (sqlite3_step(user_stmt) != SQLITE_DONE) {
            fprintf(stderr, "Insert failed: %s\n", sqlite3_errmsg(db));
            sqlite3_finalize(user_stmt);
            return -1;
        }
        sqlite3_finalize(user_stmt);

        /* Process as vendor */
        sqlite3_stmt *vendor_stmt;
        if (sqlite3_prepare_v2(db, "INSERT INTO vendors (name, email) VALUES (?, ?)",
                               -1, &vendor_stmt, NULL) != SQLITE_OK) {
            fprintf(stderr, "Prepare failed: %s\n", sqlite3_errmsg(db));
            return -1;
        }
        sqlite3_bind_text(vendor_stmt, 1, names[i], -1, SQLITE_STATIC);
        sqlite3_bind_text(vendor_stmt, 2, emails[i], -1, SQLITE_STATIC);
        if (sqlite3_step(vendor_stmt) != SQLITE_DONE) {
            fprintf(stderr, "Insert failed: %s\n", sqlite3_errmsg(db));
            sqlite3_finalize(vendor_stmt);
            return -1;
        }
        sqlite3_finalize(vendor_stmt);
    }
    return 0;
}
```

### Good Code (Fix)
```c
static int execute_insert(sqlite3 *db, const char *sql, const char *name, const char *email) {
    sqlite3_stmt *stmt;
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, NULL) != SQLITE_OK) {
        fprintf(stderr, "Prepare failed: %s\n", sqlite3_errmsg(db));
        return -1;
    }
    sqlite3_bind_text(stmt, 1, name, -1, SQLITE_STATIC);
    sqlite3_bind_text(stmt, 2, email, -1, SQLITE_STATIC);
    int rc = sqlite3_step(stmt);
    if (rc != SQLITE_DONE) {
        fprintf(stderr, "Insert failed: %s\n", sqlite3_errmsg(db));
    }
    sqlite3_finalize(stmt);
    return rc == SQLITE_DONE ? 0 : -1;
}

int process_batch(const char **names, const char **emails, int count, sqlite3 *db) {
    for (int i = 0; i < count; i++) {
        if (names[i] == NULL || emails[i] == NULL) {
            fprintf(stderr, "Skipping record %d: missing fields\n", i);
            continue;
        }
        if (execute_insert(db, "INSERT INTO users (name, email) VALUES (?, ?)",
                           names[i], emails[i]) != 0)
            return -1;
        if (execute_insert(db, "INSERT INTO vendors (name, email) VALUES (?, ?)",
                           names[i], emails[i]) != 0)
            return -1;
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `compound_statement`, `declaration`, `expression_statement`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across function bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(compound_statement
  (_) @stmt)

(function_definition
  body: (compound_statement
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
