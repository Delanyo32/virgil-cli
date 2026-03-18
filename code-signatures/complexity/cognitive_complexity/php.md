# Cognitive Complexity -- PHP

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, catch, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.php`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/catch, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```php
function processOrders(array $orders, Config $config): array
{
    $results = [];
    foreach ($orders as $order) {
        if ($order->isActive()) {
            try {
                if ($order->getType() === 'premium') {
                    foreach ($order->getItems() as $item) {
                        if (in_array($item->getSku(), $config->requiredSkus)) {
                            if ($item->getQuantity() <= 0) {
                                continue;
                            }
                            if ($item->getPrice() > $config->maxPrice) {
                                break;
                            }
                            $results[] = transformItem($item);
                        }
                    }
                } else {
                    if ($order->getFallback() !== null) {
                        $results[] = defaultTransform($order);
                    }
                }
            } catch (ValidationException $e) {
                if ($config->strict) {
                    throw $e;
                }
                $results[] = errorResult($order, $e);
            }
        }
    }
    return $results;
}
```

### Good Code (Fix)
```php
function processPremiumItems(array $items, Config $config): array
{
    $results = [];
    foreach ($items as $item) {
        if (!in_array($item->getSku(), $config->requiredSkus)) {
            continue;
        }
        if ($item->getPrice() > $config->maxPrice) {
            break;
        }
        if ($item->getQuantity() <= 0) {
            continue;
        }
        $results[] = transformItem($item);
    }
    return $results;
}

function processSingleOrder(Order $order, Config $config): ?Result
{
    if (!$order->isActive()) {
        return null;
    }
    if ($order->getType() === 'premium') {
        return compositeResult(processPremiumItems($order->getItems(), $config));
    }
    if ($order->getFallback() !== null) {
        return defaultTransform($order);
    }
    return null;
}

function processOrders(array $orders, Config $config): array
{
    $results = [];
    foreach ($orders as $order) {
        try {
            $r = processSingleOrder($order, $config);
            if ($r !== null) {
                $results[] = $r;
            }
        } catch (ValidationException $e) {
            if ($config->strict) {
                throw $e;
            }
            $results[] = errorResult($order, $e);
        }
    }
    return $results;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `foreach_statement`, `while_statement`, `do_statement`, `try_statement`, `catch_clause`, `switch_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and `throw` statements add 1 each for flow disruption. `else` clauses, `elseif` clauses, and `catch` clauses add 1 each for breaking linear flow. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_definition body: (compound_statement) @fn_body) @fn
(method_declaration body: (compound_statement) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(foreach_statement) @nesting
(while_statement) @nesting
(do_statement) @nesting
(try_statement) @nesting
(switch_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(throw_expression) @flow_break

;; Else/elseif/catch/finally break linear flow
(else_clause) @flow_break
(else_if_clause) @flow_break
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
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/catch per operation that fragments the readable logic flow.

### Bad Code (Anti-pattern)
```php
function syncUserData(string $userId): array
{
    try {
        $user = fetchUser($userId);
    } catch (ConnectionException $e) {
        return ['success' => false, 'error' => 'fetch_failed'];
    }

    try {
        $profile = fetchProfile($user->profileId);
    } catch (ConnectionException $e) {
        return ['success' => false, 'error' => 'profile_failed'];
    }

    try {
        $preferences = loadPreferences($user->id);
    } catch (FileNotFoundException $e) {
        $preferences = defaultPreferences();
    }

    try {
        $merged = mergeData($user, $profile, $preferences);
    } catch (MergeException $e) {
        return ['success' => false, 'error' => 'merge_failed'];
    }

    try {
        saveToCache($merged);
    } catch (CacheException $e) {
        error_log("Cache save failed: " . $e->getMessage());
    }

    try {
        notifyServices($merged);
    } catch (NotificationException $e) {
        return ['success' => false, 'error' => 'notify_failed'];
    }

    return ['success' => true, 'data' => $merged];
}
```

### Good Code (Fix)
```php
function loadPreferencesSafe(int $userId): Preferences
{
    try {
        return loadPreferences($userId);
    } catch (FileNotFoundException $e) {
        return defaultPreferences();
    }
}

function saveToCacheSafe(MergedData $data): void
{
    try {
        saveToCache($data);
    } catch (CacheException $e) {
        error_log("Cache save failed: " . $e->getMessage());
    }
}

function syncUserData(string $userId): array
{
    try {
        $user = fetchUser($userId);
        $profile = fetchProfile($user->profileId);
        $preferences = loadPreferencesSafe($user->id);
        $merged = mergeData($user, $profile, $preferences);

        saveToCacheSafe($merged);
        notifyServices($merged);

        return ['success' => true, 'data' => $merged];
    } catch (ConnectionException $e) {
        return ['success' => false, 'error' => identifyStage($e)];
    } catch (MergeException $e) {
        return ['success' => false, 'error' => 'merge_failed'];
    } catch (NotificationException $e) {
        return ['success' => false, 'error' => 'notify_failed'];
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`
- **Detection approach**: Count `try_statement` nodes within a single function body. If 3 or more try/catch blocks appear as siblings (not nested), flag as interleaved error handling. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a function
(function_definition
  body: (compound_statement
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
