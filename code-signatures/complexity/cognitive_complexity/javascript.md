# Cognitive Complexity -- JavaScript/TypeScript

## Overview
Cognitive complexity measures how hard code is for a human to understand, unlike cyclomatic complexity which counts paths. It penalizes nesting, breaks in linear flow (else, catch, continue, break), and recursion more heavily than simple branching.

## Why It's a Complexity Concern
High cognitive complexity code requires developers to maintain a mental stack of nested contexts. Code that is technically testable (low CC) can still be very hard to read if it has deep nesting or complex control flow interleaving.

## Applicability
- **Relevance**: high
- **Languages covered**: `.ts`, `.tsx`, `.js`, `.jsx`
- **Threshold**: 15 (typical)

---

## Pattern 1: Deep Nesting With Multiple Break Points

### Description
Functions with 3+ levels of nesting where each level introduces conditional logic, try/catch, or loop constructs, requiring the reader to track many contexts simultaneously.

### Bad Code (Anti-pattern)
```typescript
function processItems(items: Item[], config: Config): Result[] {
  const results: Result[] = [];
  for (const item of items) {
    if (item.isActive) {
      try {
        if (item.type === "premium") {
          for (const sub of item.subItems) {
            if (sub.validate()) {
              if (config.strictMode && sub.score < config.threshold) {
                continue;
              }
              results.push(transform(sub));
            } else {
              break;
            }
          }
        } else {
          if (item.fallback) {
            results.push(handleFallback(item));
          }
        }
      } catch (err) {
        if (config.abortOnError) {
          throw err;
        }
        results.push(errorResult(item, err));
      }
    }
  }
  return results;
}
```

### Good Code (Fix)
```typescript
function validateSubItem(sub: SubItem, config: Config): boolean {
  if (!sub.validate()) return false;
  if (config.strictMode && sub.score < config.threshold) return false;
  return true;
}

function processPremiumItem(item: Item, config: Config): Result[] {
  const results: Result[] = [];
  for (const sub of item.subItems) {
    if (!sub.validate()) break;
    if (!validateSubItem(sub, config)) continue;
    results.push(transform(sub));
  }
  return results;
}

function processSingleItem(item: Item, config: Config): Result[] {
  if (!item.isActive) return [];
  if (item.type === "premium") return processPremiumItem(item, config);
  if (item.fallback) return [handleFallback(item)];
  return [];
}

function processItems(items: Item[], config: Config): Result[] {
  return items.flatMap((item) => {
    try {
      return processSingleItem(item, config);
    } catch (err) {
      if (config.abortOnError) throw err;
      return [errorResult(item, err)];
    }
  });
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `for_statement`, `for_in_statement`, `while_statement`, `do_statement`, `try_statement`, `catch_clause`, `switch_statement`
- **Detection approach**: Increment a nesting counter when entering a control structure. Each decision point adds 1 + current_nesting_depth. `continue`, `break`, and early `return` statements add 1 each for flow disruption. Flag when total exceeds 15.
- **S-expression query sketch**:
```scheme
;; Find function boundaries
(function_declaration body: (statement_block) @fn_body) @fn
(method_definition body: (statement_block) @fn_body) @fn
(arrow_function body: (_) @fn_body) @fn

;; Nesting structures (each increments nesting counter)
(if_statement) @nesting
(for_statement) @nesting
(for_in_statement) @nesting
(while_statement) @nesting
(do_statement) @nesting
(try_statement) @nesting
(switch_statement) @nesting

;; Flow-breaking statements (add 1 each)
(continue_statement) @flow_break
(break_statement) @flow_break
(throw_statement) @flow_break

;; Else/else-if increments (breaks linear flow)
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
Functions that mix business logic and error handling at every step, creating a zigzag pattern of try/catch or `.catch()` chains that fragments the readable logic flow.

### Bad Code (Anti-pattern)
```typescript
async function syncUserData(userId: string): Promise<SyncResult> {
  let user;
  try {
    user = await fetchUser(userId);
  } catch (err) {
    return { success: false, error: "fetch_failed" };
  }

  let profile;
  try {
    profile = await fetchProfile(user.profileId);
  } catch (err) {
    return { success: false, error: "profile_failed" };
  }

  let preferences;
  try {
    preferences = await loadPreferences(user.id);
  } catch (err) {
    preferences = defaultPreferences();
  }

  let merged;
  try {
    merged = mergeData(user, profile, preferences);
  } catch (err) {
    return { success: false, error: "merge_failed" };
  }

  try {
    await saveToCache(merged);
  } catch (err) {
    console.warn("Cache save failed, continuing");
  }

  try {
    await notifyServices(merged);
  } catch (err) {
    return { success: false, error: "notify_failed" };
  }

  return { success: true, data: merged };
}
```

### Good Code (Fix)
```typescript
async function syncUserData(userId: string): Promise<SyncResult> {
  try {
    const user = await fetchUser(userId);
    const profile = await fetchProfile(user.profileId);
    const preferences = await loadPreferences(user.id).catch(() => defaultPreferences());
    const merged = mergeData(user, profile, preferences);

    await saveToCache(merged).catch((err) => console.warn("Cache save failed", err));
    await notifyServices(merged);

    return { success: true, data: merged };
  } catch (err) {
    const stage = identifyFailureStage(err);
    return { success: false, error: stage };
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `try_statement`, `catch_clause`, `call_expression` with `.catch` member access
- **Detection approach**: Count `try_statement` nodes within a single function body. If 3 or more try/catch blocks appear as siblings (not nested), flag as interleaved error handling. Also detect sequences of `.catch()` call chains. Each error-handling interruption adds cognitive cost proportional to its nesting depth.
- **S-expression query sketch**:
```scheme
;; Detect multiple sibling try blocks in a function
(function_declaration
  body: (statement_block
    (try_statement) @try1
    (try_statement) @try2
    (try_statement) @try3))

;; Detect .catch() chains
(call_expression
  function: (member_expression
    property: (property_identifier) @method_name
    (#eq? @method_name "catch")))
```

### Pipeline Mapping
- **Pipeline name**: `cognitive`
- **Pattern name**: `interleaved_error_handling`
- **Severity**: warning
- **Confidence**: medium
