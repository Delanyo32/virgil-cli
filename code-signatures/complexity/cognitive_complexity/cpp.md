# Cognitive Complexity -- C++

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, catch, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/catch, or loop constructs, requiring the reader to track many contexts simultaneously. C++ inherits C's nesting tendencies and adds try/catch and range-for complexity.

### Bad Code (Anti-pattern)
```cpp
std::vector<Result> processRecords(const std::vector<Record>& records, const Config& config)
{
    std::vector<Result> results;
    for (const auto& record : records) {
        if (record.isActive()) {
            try {
                if (record.category() == Category::Priority) {
                    for (const auto& field : record.fields()) {
                        if (config.requiredFields().count(field.name())) {
                            if (field.value().empty()) {
                                continue;
                            }
                            if (field.score() < config.threshold()) {
                                if (config.strict()) {
                                    throw StrictViolation(field.name());
                                }
                                break;
                            }
                            results.push_back(transformField(field));
                        }
                    }
                } else {
                    if (record.fallback().has_value()) {
                        results.push_back(defaultTransform(record));
                    }
                }
            } catch (const ValidationException& e) {
                if (config.strict()) {
                    throw;
                }
                results.push_back(errorResult(record, e));
            }
        }
    }
    return results;
}
```

### Good Code (Fix)
```cpp
std::vector<Result> processPriorityFields(const std::vector<Field>& fields, const Config& config)
{
    std::vector<Result> results;
    for (const auto& field : fields) {
        if (!config.requiredFields().count(field.name())) continue;
        if (field.value().empty()) continue;

        if (field.score() < config.threshold()) {
            if (config.strict()) throw StrictViolation(field.name());
            break;
        }
        results.push_back(transformField(field));
    }
    return results;
}

std::optional<Result> processSingleRecord(const Record& record, const Config& config)
{
    if (!record.isActive()) return std::nullopt;
    if (record.category() == Category::Priority) {
        auto partial = processPriorityFields(record.fields(), config);
        return compositeResult(std::move(partial));
    }
    if (record.fallback().has_value()) {
        return defaultTransform(record);
    }
    return std::nullopt;
}

std::vector<Result> processRecords(const std::vector<Record>& records, const Config& config)
{
    std::vector<Result> results;
    for (const auto& record : records) {
        try {
            auto r = processSingleRecord(record, config);
            if (r) results.push_back(std::move(*r));
        } catch (const ValidationException& e) {
            if (config.strict()) throw;
            results.push_back(errorResult(record, e));
        }
    }
    return results;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `for_range_loop`, `while_statement`, `do_statement`, `try_statement`, `catch_clause`, `switch_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, `goto`, `throw`, and early `return` statements add 1 each for flow disruption. `else` clauses and `catch` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_definition body: (compound_statement) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(for_range_loop) @nesting
(while_statement) @nesting
(do_statement) @nesting
(try_statement) @nesting
(switch_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(goto_statement) @flow_break
(throw_statement) @flow_break
(return_statement) @flow_break

;; Else/catch break linear flow
(else_clause) @flow_break
(catch_clause) @flow_break
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/catch per operation or repeated return-code checks. C++ codebases often combine both exception-based and return-code-based error handling, making the interleaving especially hard to follow.

### Bad Code (Anti-pattern)
```cpp
SyncResult syncUserData(const std::string& userId)
{
    std::unique_ptr<User> user;
    try {
        user = fetchUser(userId);
    } catch (const ConnectionException& e) {
        return SyncResult::failure("fetch_failed");
    }

    std::unique_ptr<Profile> profile;
    try {
        profile = fetchProfile(user->profileId());
    } catch (const ConnectionException& e) {
        return SyncResult::failure("profile_failed");
    }

    Preferences prefs;
    try {
        prefs = loadPreferences(user->id());
    } catch (const FileNotFoundException& e) {
        prefs = Preferences::defaults();
    }

    std::unique_ptr<MergedData> merged;
    try {
        merged = mergeData(*user, *profile, prefs);
    } catch (const MergeException& e) {
        return SyncResult::failure("merge_failed");
    }

    try {
        saveToCache(*merged);
    } catch (const CacheException& e) {
        std::cerr << "Cache save failed: " << e.what() << '\n';
    }

    try {
        notifyServices(*merged);
    } catch (const NotificationException& e) {
        return SyncResult::failure("notify_failed");
    }

    return SyncResult::success(std::move(merged));
}
```

### Good Code (Fix)
```cpp
Preferences loadPreferencesSafe(int userId)
{
    try {
        return loadPreferences(userId);
    } catch (const FileNotFoundException&) {
        return Preferences::defaults();
    }
}

void saveToCacheSafe(const MergedData& data)
{
    try {
        saveToCache(data);
    } catch (const CacheException& e) {
        std::cerr << "Cache save failed: " << e.what() << '\n';
    }
}

SyncResult syncUserData(const std::string& userId)
{
    try {
        auto user = fetchUser(userId);
        auto profile = fetchProfile(user->profileId());
        auto prefs = loadPreferencesSafe(user->id());
        auto merged = mergeData(*user, *profile, prefs);

        saveToCacheSafe(*merged);
        notifyServices(*merged);

        return SyncResult::success(std::move(merged));
    } catch (const ConnectionException& e) {
        return SyncResult::failure(identifyStage(e));
    } catch (const MergeException&) {
        return SyncResult::failure("merge_failed");
    } catch (const NotificationException&) {
        return SyncResult::failure("notify_failed");
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `if_statement` with return-code checks
- **Detection approach**: Count `try_statement` nodes within a single function body. If 3 or more try/catch blocks appear as siblings (not nested), flag as interleaved error handling. Also detect C-style return-code checking patterns (if statements testing return values against 0, -1, nullptr). Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a function
(function_definition
  body: (compound_statement
    (try_statement) @try1
    (try_statement) @try2
    (try_statement) @try3))

;; Detect C-style nullptr checks interleaved with logic
(if_statement
  condition: (binary_expression
    operator: "=="
    right: (nullptr))) @null_check

;; Detect catch clauses
(catch_clause) @error_handler
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
