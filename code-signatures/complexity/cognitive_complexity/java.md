# Cognitive Complexity -- Java

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, catch, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.java`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/catch, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```java
public List<Result> processOrders(List<Order> orders, Config config) {
    List<Result> results = new ArrayList<>();
    for (Order order : orders) {
        if (order.isActive()) {
            try {
                if (order.getType() == OrderType.PREMIUM) {
                    for (LineItem item : order.getItems()) {
                        if (config.getRequiredSkus().contains(item.getSku())) {
                            if (item.getQuantity() <= 0) {
                                continue;
                            }
                            if (item.getPrice() > config.getMaxPrice()) {
                                break;
                            }
                            results.add(transformItem(item));
                        }
                    }
                } else {
                    if (order.getFallback() != null) {
                        results.add(defaultTransform(order));
                    }
                }
            } catch (ValidationException e) {
                if (config.isStrict()) {
                    throw e;
                }
                results.add(errorResult(order, e));
            }
        }
    }
    return results;
}
```

### Good Code (Fix)
```java
private boolean isUsableItem(LineItem item, Config config) {
    return item.getQuantity() > 0 && item.getPrice() <= config.getMaxPrice();
}

private List<Result> processPremiumItems(List<LineItem> items, Config config) {
    List<Result> results = new ArrayList<>();
    for (LineItem item : items) {
        if (!config.getRequiredSkus().contains(item.getSku())) {
            continue;
        }
        if (item.getPrice() > config.getMaxPrice()) {
            break;
        }
        if (item.getQuantity() <= 0) {
            continue;
        }
        results.add(transformItem(item));
    }
    return results;
}

private Result processSingleOrder(Order order, Config config) {
    if (!order.isActive()) return null;
    if (order.getType() == OrderType.PREMIUM) {
        return compositeResult(processPremiumItems(order.getItems(), config));
    }
    if (order.getFallback() != null) {
        return defaultTransform(order);
    }
    return null;
}

public List<Result> processOrders(List<Order> orders, Config config) {
    List<Result> results = new ArrayList<>();
    for (Order order : orders) {
        try {
            Result r = processSingleOrder(order, config);
            if (r != null) results.add(r);
        } catch (ValidationException e) {
            if (config.isStrict()) throw e;
            results.add(errorResult(order, e));
        }
    }
    return results;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement`, `try_statement`, `catch_clause`, `switch_expression`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and `throw` statements add 1 each for flow disruption. `else` clauses and `catch` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find method boundaries
(method_declaration body: (block) @fn_body) @fn
(constructor_declaration body: (constructor_body) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(enhanced_for_statement) @nesting
(while_statement) @nesting
(do_statement) @nesting
(try_statement) @nesting
(switch_expression) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(throw_statement) @flow_break

;; Else/catch/finally break linear flow
(if_statement
  alternative: (_) @else_branch)
(catch_clause) @flow_break
(finally_clause) @flow_break
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `deep_nesting_flow_breaks`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Interleaved Logic and Error Handling

### Description
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/catch per operation that fragments the readable logic flow. In Java, this often manifests as multiple sequential try/catch blocks wrapping individual method calls.

### Bad Code (Anti-pattern)
```java
public SyncResult syncUserData(String userId) {
    User user;
    try {
        user = fetchUser(userId);
    } catch (ConnectionException e) {
        return SyncResult.failure("fetch_failed");
    }

    Profile profile;
    try {
        profile = fetchProfile(user.getProfileId());
    } catch (ConnectionException e) {
        return SyncResult.failure("profile_failed");
    }

    Preferences preferences;
    try {
        preferences = loadPreferences(user.getId());
    } catch (FileNotFoundException e) {
        preferences = Preferences.defaults();
    }

    MergedData merged;
    try {
        merged = mergeData(user, profile, preferences);
    } catch (MergeException e) {
        return SyncResult.failure("merge_failed");
    }

    try {
        saveToCache(merged);
    } catch (CacheException e) {
        logger.warn("Cache save failed", e);
    }

    try {
        notifyServices(merged);
    } catch (NotificationException e) {
        return SyncResult.failure("notify_failed");
    }

    return SyncResult.success(merged);
}
```

### Good Code (Fix)
```java
public SyncResult syncUserData(String userId) {
    try {
        User user = fetchUser(userId);
        Profile profile = fetchProfile(user.getProfileId());
        Preferences preferences = loadPreferencesSafe(user.getId());
        MergedData merged = mergeData(user, profile, preferences);

        saveToCacheSafe(merged);
        notifyServices(merged);

        return SyncResult.success(merged);
    } catch (ConnectionException e) {
        return SyncResult.failure(identifyStage(e));
    } catch (MergeException e) {
        return SyncResult.failure("merge_failed");
    } catch (NotificationException e) {
        return SyncResult.failure("notify_failed");
    }
}

private Preferences loadPreferencesSafe(long userId) {
    try {
        return loadPreferences(userId);
    } catch (FileNotFoundException e) {
        return Preferences.defaults();
    }
}

private void saveToCacheSafe(MergedData data) {
    try {
        saveToCache(data);
    } catch (CacheException e) {
        logger.warn("Cache save failed", e);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`
- **Detection approach**: Count `try_statement` nodes within a single method body. If 3 or more try/catch blocks appear as siblings (not nested), flag as interleaved error handling. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a method
(method_declaration
  body: (block
    (try_statement) @try1
    (try_statement) @try2
    (try_statement) @try3))

;; Detect catch clauses
(catch_clause) @error_handler
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
