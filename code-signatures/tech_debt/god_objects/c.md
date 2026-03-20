# God Objects -- C

## Overview
God objects in C manifest as source files with too many functions (50+), oversized structs that hold unrelated data, or single `.c` files that serve as a module handling multiple unrelated concerns. Since C lacks classes, the "file as module" pattern is the primary unit of encapsulation, and oversized modules violate the Single Responsibility Principle.

## Why It's a Tech Debt Concern
Oversized C source files become merge-conflict hotspots when multiple developers need to add or modify functions in the same file. Compilation times increase because any change to a large `.c` file triggers recompilation of the entire translation unit. Understanding the relationships between 50+ functions in a single file creates significant cognitive load, and static analysis tools struggle with overly complex modules.

## Applicability
- **Relevance**: medium (C's file-as-module pattern makes "god modules" the equivalent of god classes)
- **Languages covered**: `.c`, `.h`
- **Frameworks/libraries**: Linux kernel (oversized driver files), embedded systems (monolithic main.c), GTK/GLib (oversized object implementations)

---

## Pattern 1: Oversized Source File / Struct

### Description
A source file with 50+ functions acting as a single module, or a struct with 20+ fields holding data for multiple unrelated concerns. The file contains functions spanning user management, I/O, parsing, memory management, and error handling all in one translation unit.

### Bad Code (Anti-pattern)
```c
/* app_manager.c - handles everything */

typedef struct {
    /* Database fields */
    sqlite3 *db;
    sqlite3_stmt *insert_stmt;
    sqlite3_stmt *select_stmt;
    sqlite3_stmt *update_stmt;
    sqlite3_stmt *delete_stmt;
    /* Network fields */
    int server_fd;
    int client_fds[MAX_CLIENTS];
    int num_clients;
    struct sockaddr_in server_addr;
    /* Config fields */
    char config_path[PATH_MAX];
    char log_path[PATH_MAX];
    int log_level;
    int port;
    int max_connections;
    /* Cache fields */
    hash_table_t *cache;
    int cache_size;
    int cache_ttl;
    /* Auth fields */
    char secret_key[256];
    int token_expiry;
} AppManager;

/* Database functions */
int app_manager_init_db(AppManager *mgr, const char *path);
int app_manager_close_db(AppManager *mgr);
int app_manager_insert_user(AppManager *mgr, const User *user);
int app_manager_update_user(AppManager *mgr, int id, const User *user);
int app_manager_delete_user(AppManager *mgr, int id);
User *app_manager_find_user(AppManager *mgr, int id);
User **app_manager_list_users(AppManager *mgr, int *count);

/* Network functions */
int app_manager_start_server(AppManager *mgr);
int app_manager_stop_server(AppManager *mgr);
int app_manager_accept_client(AppManager *mgr);
int app_manager_handle_request(AppManager *mgr, int client_fd);
int app_manager_send_response(AppManager *mgr, int client_fd, const char *data);
int app_manager_broadcast(AppManager *mgr, const char *message);

/* Validation functions */
int app_manager_validate_email(const char *email);
int app_manager_validate_password(const char *password);
int app_manager_validate_input(const char *input, int max_len);

/* Auth functions */
char *app_manager_generate_token(AppManager *mgr, const User *user);
int app_manager_verify_token(AppManager *mgr, const char *token);
int app_manager_hash_password(const char *password, char *out, size_t out_len);
int app_manager_check_permission(AppManager *mgr, int user_id, const char *resource);

/* Cache functions */
int app_manager_cache_set(AppManager *mgr, const char *key, const void *value, size_t len);
void *app_manager_cache_get(AppManager *mgr, const char *key);
int app_manager_cache_invalidate(AppManager *mgr, const char *key);
int app_manager_cache_flush(AppManager *mgr);

/* Config functions */
int app_manager_load_config(AppManager *mgr, const char *path);
int app_manager_save_config(AppManager *mgr);
const char *app_manager_get_config(AppManager *mgr, const char *key);

/* Logging functions */
void app_manager_log(AppManager *mgr, int level, const char *fmt, ...);
int app_manager_rotate_logs(AppManager *mgr);
int app_manager_set_log_level(AppManager *mgr, int level);

/* Utility functions */
char *app_manager_format_date(time_t timestamp);
int app_manager_parse_json(const char *json, JsonValue *out);
char *app_manager_to_json(const JsonValue *value);
int app_manager_send_email(AppManager *mgr, const char *to, const char *subject, const char *body);
int app_manager_generate_report(AppManager *mgr, const char *output_path);
int app_manager_export_data(AppManager *mgr, const char *format, const char *path);
int app_manager_import_data(AppManager *mgr, const char *path);
/* ... 15 more functions ... */
```

### Good Code (Fix)
```c
/* user_db.c - user database operations */
typedef struct {
    sqlite3 *db;
    sqlite3_stmt *insert_stmt;
    sqlite3_stmt *select_stmt;
    sqlite3_stmt *update_stmt;
    sqlite3_stmt *delete_stmt;
} UserDB;

int user_db_init(UserDB *udb, const char *path);
int user_db_close(UserDB *udb);
int user_db_insert(UserDB *udb, const User *user);
int user_db_update(UserDB *udb, int id, const User *user);
int user_db_delete(UserDB *udb, int id);
User *user_db_find(UserDB *udb, int id);

/* server.c - network server */
typedef struct {
    int server_fd;
    int client_fds[MAX_CLIENTS];
    int num_clients;
    struct sockaddr_in addr;
} Server;

int server_start(Server *srv, int port);
int server_stop(Server *srv);
int server_accept(Server *srv);
int server_handle_request(Server *srv, int client_fd);

/* auth.c - authentication */
typedef struct {
    char secret_key[256];
    int token_expiry;
} AuthContext;

char *auth_generate_token(AuthContext *ctx, const User *user);
int auth_verify_token(AuthContext *ctx, const char *token);
int auth_hash_password(const char *password, char *out, size_t out_len);

/* cache.c - caching layer */
typedef struct {
    hash_table_t *table;
    int max_size;
    int ttl;
} Cache;

int cache_set(Cache *c, const char *key, const void *value, size_t len);
void *cache_get(Cache *c, const char *key);
int cache_invalidate(Cache *c, const char *key);
int cache_flush(Cache *c);
```

### Tree-sitter Detection Strategy
- **Target node types**: `translation_unit` (count `function_definition` children), `struct_specifier` (count `field_declaration` children in `field_declaration_list`)
- **Detection approach**: Count `function_definition` nodes at the top level of a `translation_unit` (source file). Flag when function count exceeds 30 for a single `.c` file. For structs, count `field_declaration` nodes in `field_declaration_list` and flag when fields exceed 15.
- **S-expression query sketch**:
  ```scheme
  (translation_unit
    (function_definition
      declarator: (function_declarator
        declarator: (identifier) @func_name)))

  (struct_specifier
    name: (type_identifier) @struct_name
    body: (field_declaration_list
      (field_declaration) @field))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single source file or struct with functions spanning data access, network I/O, business logic, and utility operations — a clear SRP violation. In C, this manifests as a `.c` file where functions from 4+ different concern areas coexist, making it hard to understand or modify any single concern without reading the entire file.

### Bad Code (Anti-pattern)
```c
/* order_manager.c */

/* HTTP handling */
int order_handle_create(int client_fd, const char *body) { /* ... */ }
int order_handle_get(int client_fd, int order_id) { /* ... */ }
int order_handle_list(int client_fd, const char *query) { /* ... */ }

/* Validation */
int order_validate_items(const OrderItem *items, int count) { /* ... */ }
int order_validate_coupon(const char *code) { /* ... */ }

/* Business logic */
double order_calculate_total(const OrderItem *items, int count) { /* ... */ }
double order_calculate_tax(double subtotal) { /* ... */ }
double order_calculate_shipping(const OrderItem *items, int count, const Address *addr) { /* ... */ }

/* Database access */
int order_save(sqlite3 *db, const Order *order) { /* ... */ }
int order_save_items(sqlite3 *db, int order_id, const OrderItem *items, int count) { /* ... */ }
int order_update_inventory(sqlite3 *db, const OrderItem *items, int count) { /* ... */ }
Order *order_find_by_id(sqlite3 *db, int id) { /* ... */ }

/* Notifications */
int order_send_confirmation(const char *email, const Order *order) { /* ... */ }
int order_notify_warehouse(const Order *order) { /* ... */ }

/* Logging */
void order_log_event(int order_id, const char *event) { /* ... */ }
void order_track_metric(const char *metric, double value) { /* ... */ }
```

### Good Code (Fix)
```c
/* order_handler.c - HTTP layer only */
int order_handle_create(int client_fd, const char *body) {
    OrderCreateReq req;
    order_parse_request(body, &req);
    Order order;
    int err = order_service_create(&req, &order);
    if (err) return order_send_error(client_fd, err);
    return order_send_json(client_fd, &order);
}

/* order_service.c - business logic */
int order_service_create(const OrderCreateReq *req, Order *out) {
    double total = pricing_calculate(req->items, req->item_count, req->coupon);
    int err = order_repo_save(req->items, req->item_count, total, out);
    if (err) return err;
    inventory_deduct(req->items, req->item_count);
    notification_order_confirmed(req->email, out);
    return 0;
}

/* order_repo.c - database access */
int order_repo_save(const OrderItem *items, int count, double total, Order *out) { /* ... */ }
Order *order_repo_find_by_id(int id) { /* ... */ }

/* notification.c - email/messaging */
int notification_order_confirmed(const char *email, const Order *order) { /* ... */ }
int notification_warehouse(const Order *order) { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `translation_unit`, `function_definition` — heuristic based on function name prefixes
- **Detection approach**: Group functions by name prefix pattern (e.g., `order_handle_` = HTTP, `order_validate_` = validation, `order_save_`/`order_find_` = persistence, `order_send_`/`order_notify_` = communication, `order_log_`/`order_track_` = observability, `order_calculate_` = business logic). Flag files where functions span 4+ categories based on common prefix grouping.
- **S-expression query sketch**:
  ```scheme
  (translation_unit
    (function_definition
      declarator: (function_declarator
        declarator: (identifier) @func_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
