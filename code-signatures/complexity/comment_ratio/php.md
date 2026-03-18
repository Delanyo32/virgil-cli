# Comment Ratio -- PHP

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .php
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```php
function applyPromotions(array $cart, array $promotions, ?User $user): array
{
    $applied = [];
    foreach ($promotions as $promo) {
        if ($promo->expiresAt && $promo->expiresAt < new \DateTime()) {
            continue;
        }
        if ($promo->minCartValue && $cart['subtotal'] < $promo->minCartValue) {
            continue;
        }
        if ($promo->userSegment !== null) {
            if ($user === null || !in_array($promo->userSegment, $user->getSegments())) {
                continue;
            }
        }
        if ($promo->type === 'percentage') {
            $discount = $cart['subtotal'] * ($promo->value / 100);
            if ($promo->maxDiscount && $discount > $promo->maxDiscount) {
                $discount = $promo->maxDiscount;
            }
            $cart['subtotal'] -= $discount;
            $applied[] = ['promo' => $promo->code, 'discount' => $discount];
        } elseif ($promo->type === 'fixed') {
            $cart['subtotal'] -= $promo->value;
            $applied[] = ['promo' => $promo->code, 'discount' => $promo->value];
        } elseif ($promo->type === 'bogo') {
            $cheapest = PHP_FLOAT_MAX;
            foreach ($cart['items'] as $item) {
                if ($item['price'] < $cheapest) {
                    $cheapest = $item['price'];
                }
            }
            $cart['subtotal'] -= $cheapest;
            $applied[] = ['promo' => $promo->code, 'discount' => $cheapest];
        }
        if (!$promo->stackable) {
            break;
        }
    }
    $cart['promotions'] = $applied;
    return $cart;
}
```

### Good Code (Fix)
```php
/**
 * Applies eligible promotions to the cart in priority order.
 * Non-stackable promotions stop further processing after they apply.
 */
function applyPromotions(array $cart, array $promotions, ?User $user): array
{
    $applied = [];
    foreach ($promotions as $promo) {
        if ($promo->expiresAt && $promo->expiresAt < new \DateTime()) {
            continue;
        }
        if ($promo->minCartValue && $cart['subtotal'] < $promo->minCartValue) {
            continue;
        }
        // Segment-restricted promos require an authenticated user belonging
        // to the target segment (e.g., "loyalty_gold", "employee")
        if ($promo->userSegment !== null) {
            if ($user === null || !in_array($promo->userSegment, $user->getSegments())) {
                continue;
            }
        }

        if ($promo->type === 'percentage') {
            $discount = $cart['subtotal'] * ($promo->value / 100);
            // Cap prevents abuse on high-value carts with aggressive % codes
            if ($promo->maxDiscount && $discount > $promo->maxDiscount) {
                $discount = $promo->maxDiscount;
            }
            $cart['subtotal'] -= $discount;
            $applied[] = ['promo' => $promo->code, 'discount' => $discount];
        } elseif ($promo->type === 'fixed') {
            $cart['subtotal'] -= $promo->value;
            $applied[] = ['promo' => $promo->code, 'discount' => $promo->value];
        } elseif ($promo->type === 'bogo') {
            // BOGO: free item is the cheapest in the cart, not the most expensive,
            // to minimize revenue impact per finance team policy
            $cheapest = PHP_FLOAT_MAX;
            foreach ($cart['items'] as $item) {
                if ($item['price'] < $cheapest) {
                    $cheapest = $item['price'];
                }
            }
            $cart['subtotal'] -= $cheapest;
            $applied[] = ['promo' => $promo->code, 'discount' => $cheapest];
        }

        // Non-stackable promos are exclusive -- only one can apply per order
        if (!$promo->stackable) {
            break;
        }
    }
    $cart['promotions'] = $applied;
    return $cart;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `method_declaration` for function bodies; `comment` for `//`, `#`, `/* */`, `/** */` (PHPDoc)
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold. Consider PHPDoc blocks above the function signature as part of the function's documentation.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_definition
    body: (compound_statement) @function.body)

  (method_declaration
    body: (compound_statement) @function.body)

  (comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```php
function deleteUser(int $userId): bool
{
    // Find the user by ID
    $user = User::find($userId);

    // Check if user is null
    if ($user === null) {
        // Return false
        return false;
    }

    // Delete the user's posts
    $user->posts()->delete();

    // Delete the user's comments
    $user->comments()->delete();

    // Delete the user
    $user->delete();

    // Return true
    return true;
}
```

### Good Code (Fix)
```php
function deleteUser(int $userId): bool
{
    $user = User::find($userId);
    if ($user === null) {
        return false;
    }

    // Cascade manually -- soft-delete models bypass DB-level ON DELETE CASCADE
    $user->posts()->delete();
    $user->comments()->delete();
    $user->delete();

    return true;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `expression_statement`, `return_statement`, `if_statement`, `echo_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (compound_statement
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
