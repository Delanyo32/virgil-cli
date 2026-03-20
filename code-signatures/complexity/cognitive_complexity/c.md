# Cognitive Complexity -- C

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, continue, break, goto), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c`, `.h`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, loops, or switch constructs, requiring the reader to track many contexts simultaneously. C code is particularly prone to this due to manual resource management and the absence of exceptions.

### Bad Code (Anti-pattern)
```c
int process_records(Record *records, int count, Config *config, Result **out, int *out_count)
{
    *out_count = 0;
    for (int i = 0; i < count; i++) {
        if (records[i].is_active) {
            switch (records[i].category) {
            case CATEGORY_PRIORITY:
                for (int j = 0; j < records[i].field_count; j++) {
                    if (is_required_field(config, records[i].fields[j].name)) {
                        if (records[i].fields[j].value == NULL) {
                            continue;
                        }
                        if (records[i].fields[j].score < config->threshold) {
                            if (config->strict) {
                                goto cleanup;
                            }
                            break;
                        }
                        out[*out_count] = transform_field(&records[i].fields[j]);
                        if (out[*out_count] == NULL) {
                            return -1;
                        }
                        (*out_count)++;
                    }
                }
                break;
            case CATEGORY_STANDARD:
                if (records[i].fallback != NULL) {
                    out[*out_count] = default_transform(&records[i]);
                    (*out_count)++;
                }
                break;
            }
        }
    }
    return 0;

cleanup:
    free_results(out, *out_count);
    return -2;
}
```

### Good Code (Fix)
```c
static int process_priority_fields(Field *fields, int field_count, Config *config,
                                    Result **out, int *out_count)
{
    for (int j = 0; j < field_count; j++) {
        if (!is_required_field(config, fields[j].name)) continue;
        if (fields[j].value == NULL) continue;

        if (fields[j].score < config->threshold) {
            if (config->strict) return -2;
            break;
        }

        out[*out_count] = transform_field(&fields[j]);
        if (out[*out_count] == NULL) return -1;
        (*out_count)++;
    }
    return 0;
}

static int process_single_record(Record *record, Config *config, Result **out, int *out_count)
{
    if (!record->is_active) return 0;

    switch (record->category) {
    case CATEGORY_PRIORITY:
        return process_priority_fields(record->fields, record->field_count, config, out, out_count);
    case CATEGORY_STANDARD:
        if (record->fallback != NULL) {
            out[*out_count] = default_transform(record);
            (*out_count)++;
        }
        return 0;
    default:
        return 0;
    }
}

int process_records(Record *records, int count, Config *config, Result **out, int *out_count)
{
    *out_count = 0;
    for (int i = 0; i < count; i++) {
        int rc = process_single_record(&records[i], config, out, out_count);
        if (rc != 0) {
            free_results(out, *out_count);
            return rc;
        }
    }
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `while_statement`, `do_statement`, `switch_statement`, `case_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, `goto`, and early `return` statements add 1 each for flow disruption. `else` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_definition body: (compound_statement) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(while_statement) @nesting
(do_statement) @nesting
(switch_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(goto_statement) @flow_break
(return_statement) @flow_break

;; Else clauses break linear flow
(else_clause) @flow_break
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and return-code checking at every step, creating a zigzag pattern of `if (rc != 0)` or `if (ptr == NULL)` checks that fragments the readable logic flow. This is C's most common source of high cognitive complexity since there are no exceptions.

### Bad Code (Anti-pattern)
```c
int sync_user_data(const char *user_id, SyncResult *result)
{
    User *user = fetch_user(user_id);
    if (user == NULL) {
        result->error = "fetch_failed";
        return -1;
    }

    Profile *profile = fetch_profile(user->profile_id);
    if (profile == NULL) {
        free_user(user);
        result->error = "profile_failed";
        return -1;
    }

    Preferences *prefs = load_preferences(user->id);
    if (prefs == NULL) {
        prefs = default_preferences();
        if (prefs == NULL) {
            free_profile(profile);
            free_user(user);
            result->error = "alloc_failed";
            return -1;
        }
    }

    MergedData *merged = merge_data(user, profile, prefs);
    if (merged == NULL) {
        free_preferences(prefs);
        free_profile(profile);
        free_user(user);
        result->error = "merge_failed";
        return -1;
    }

    int rc = save_to_cache(merged);
    if (rc != 0) {
        fprintf(stderr, "Cache save failed: %d\n", rc);
    }

    rc = notify_services(merged);
    if (rc != 0) {
        free_merged(merged);
        free_preferences(prefs);
        free_profile(profile);
        free_user(user);
        result->error = "notify_failed";
        return -1;
    }

    result->data = merged;
    free_preferences(prefs);
    free_profile(profile);
    free_user(user);
    return 0;
}
```

### Good Code (Fix)
```c
typedef struct {
    User *user;
    Profile *profile;
    Preferences *prefs;
    MergedData *merged;
} SyncContext;

static void sync_context_cleanup(SyncContext *ctx)
{
    if (ctx->merged) free_merged(ctx->merged);
    if (ctx->prefs) free_preferences(ctx->prefs);
    if (ctx->profile) free_profile(ctx->profile);
    if (ctx->user) free_user(ctx->user);
}

static int sync_load_data(SyncContext *ctx, const char *user_id)
{
    ctx->user = fetch_user(user_id);
    if (!ctx->user) return -1;

    ctx->profile = fetch_profile(ctx->user->profile_id);
    if (!ctx->profile) return -2;

    ctx->prefs = load_preferences(ctx->user->id);
    if (!ctx->prefs) ctx->prefs = default_preferences();
    if (!ctx->prefs) return -3;

    ctx->merged = merge_data(ctx->user, ctx->profile, ctx->prefs);
    return ctx->merged ? 0 : -4;
}

int sync_user_data(const char *user_id, SyncResult *result)
{
    SyncContext ctx = {0};

    int rc = sync_load_data(&ctx, user_id);
    if (rc != 0) {
        static const char *stages[] = {NULL, "fetch_failed", "profile_failed", "alloc_failed", "merge_failed"};
        result->error = stages[-rc];
        sync_context_cleanup(&ctx);
        return -1;
    }

    if (save_to_cache(ctx.merged) != 0) {
        fprintf(stderr, "Cache save failed\n");
    }

    if (notify_services(ctx.merged) != 0) {
        result->error = "notify_failed";
        sync_context_cleanup(&ctx);
        return -1;
    }

    result->data = ctx.merged;
    ctx.merged = NULL;  /* transfer ownership */
    sync_context_cleanup(&ctx);
    return 0;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `return_statement`, `call_expression`
- **Detection approach**: Detect the idiomatic C error-check pattern: a variable assignment followed by an `if_statement` that tests `== NULL`, `!= NULL`, `== -1`, `!= 0`, or `< 0`. Count these pairs within a single function body. If 4 or more appear as siblings, flag as interleaved error handling. Each error check adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect NULL-check pattern
(if_statement
  condition: (binary_expression
    left: (identifier) @var
    operator: "=="
    right: (null))) @null_check

;; Detect return-code check pattern
(if_statement
  condition: (binary_expression
    left: (identifier) @var
    operator: "!="
    right: (number_literal) @val
    (#eq? @val "0"))) @rc_check

;; Count multiple error checks in a function
(function_definition
  body: (compound_statement
    (if_statement) @check1
    (if_statement) @check2
    (if_statement) @check3
    (if_statement) @check4))
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
