# Duplicate Code -- PHP

## Overview
Duplicate code (code clones) occurs when similar or identical logic appears in multiple locations. This violates the DRY (Don't Repeat Yourself) principle and creates maintenance hazards where fixes must be applied in multiple places.

## Why It's a Code Style Concern
Bug fixes applied to one copy but not the other create inconsistencies. Feature changes require updating every copy. Duplicated code inflates codebase size, increases review burden, and often signals missing abstractions.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Frameworks/libraries**: N/A

---

## Pattern 1: Copy-Pasted Function Bodies

### Description
Two or more methods with near-identical bodies, differing only in variable names or minor constants — candidates for extraction into a shared method with parameters. Common in duplicate controller methods.

### Bad Code (Anti-pattern)
```php
class OrderController extends Controller
{
    public function storeUserOrder(Request $request)
    {
        $validated = $request->validate([
            'name' => 'required|string|max:255',
            'email' => 'required|email',
            'quantity' => 'required|integer|min:1',
            'price' => 'required|numeric|min:0',
        ]);
        $normalizedName = strtolower(trim($validated['name']));
        $amount = $validated['quantity'] * $validated['price'];
        $tax = $amount * 0.08;
        $order = UserOrder::create([
            'name' => $normalizedName,
            'email' => $validated['email'],
            'amount' => $amount,
            'tax' => $tax,
            'created_at' => now(),
        ]);
        Mail::to($validated['email'])->send(new OrderConfirmation($order));
        return response()->json($order, 201);
    }

    public function storeAdminOrder(Request $request)
    {
        $validated = $request->validate([
            'name' => 'required|string|max:255',
            'email' => 'required|email',
            'quantity' => 'required|integer|min:1',
            'price' => 'required|numeric|min:0',
        ]);
        $normalizedName = strtolower(trim($validated['name']));
        $amount = $validated['quantity'] * $validated['price'];
        $tax = $amount * 0.08;
        $order = AdminOrder::create([
            'name' => $normalizedName,
            'email' => $validated['email'],
            'amount' => $amount,
            'tax' => $tax,
            'created_at' => now(),
        ]);
        Mail::to($validated['email'])->send(new OrderConfirmation($order));
        return response()->json($order, 201);
    }
}
```

### Good Code (Fix)
```php
class OrderController extends Controller
{
    private function createOrder(Request $request, string $modelClass)
    {
        $validated = $request->validate([
            'name' => 'required|string|max:255',
            'email' => 'required|email',
            'quantity' => 'required|integer|min:1',
            'price' => 'required|numeric|min:0',
        ]);
        $normalizedName = strtolower(trim($validated['name']));
        $amount = $validated['quantity'] * $validated['price'];
        $tax = $amount * 0.08;
        $order = $modelClass::create([
            'name' => $normalizedName,
            'email' => $validated['email'],
            'amount' => $amount,
            'tax' => $tax,
            'created_at' => now(),
        ]);
        Mail::to($validated['email'])->send(new OrderConfirmation($order));
        return response()->json($order, 201);
    }

    public function storeUserOrder(Request $request)
    {
        return $this->createOrder($request, UserOrder::class);
    }

    public function storeAdminOrder(Request $request)
    {
        return $this->createOrder($request, AdminOrder::class);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`, `function_definition`, `compound_statement`
- **Detection approach**: Hash normalized function bodies (strip variable names, normalize whitespace). Functions with identical or near-identical hashes are clones. Also compare AST subtree structure — two functions with identical node-type sequences but different identifiers are Type-2 clones.
- **S-expression query sketch**:
```scheme
(method_declaration
  name: (name) @func_name
  body: (compound_statement) @func_body)

(function_definition
  name: (name) @func_name
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
The same sequence of 5+ statements repeated within a method or across methods in the same class, often due to copy-paste during development. Common in duplicate controller methods and repeated Eloquent queries.

### Bad Code (Anti-pattern)
```php
class ReportService
{
    public function generateSalesReport($startDate, $endDate)
    {
        $records = Sale::whereBetween('date', [$startDate, $endDate])->get();
        $filtered = $records->filter(fn($r) => $r->amount > 0);
        $grouped = $filtered->groupBy(fn($r) => $r->date->format('Y-m'));
        $summaries = $grouped->map(function ($group, $month) {
            return [
                'month' => $month,
                'total' => $group->sum('amount'),
                'average' => $group->avg('amount'),
                'count' => $group->count(),
            ];
        })->values();
        Cache::put("sales_{$startDate}_{$endDate}", $summaries, now()->addMinutes(30));
        return $summaries;
    }

    public function generateReturnsReport($startDate, $endDate)
    {
        $records = ReturnItem::whereBetween('date', [$startDate, $endDate])->get();
        $filtered = $records->filter(fn($r) => $r->amount > 0);
        $grouped = $filtered->groupBy(fn($r) => $r->date->format('Y-m'));
        $summaries = $grouped->map(function ($group, $month) {
            return [
                'month' => $month,
                'total' => $group->sum('amount'),
                'average' => $group->avg('amount'),
                'count' => $group->count(),
            ];
        })->values();
        Cache::put("returns_{$startDate}_{$endDate}", $summaries, now()->addMinutes(30));
        return $summaries;
    }
}
```

### Good Code (Fix)
```php
class ReportService
{
    private function buildPeriodSummary(string $modelClass, $startDate, $endDate, string $cachePrefix)
    {
        $records = $modelClass::whereBetween('date', [$startDate, $endDate])->get();
        $filtered = $records->filter(fn($r) => $r->amount > 0);
        $grouped = $filtered->groupBy(fn($r) => $r->date->format('Y-m'));
        $summaries = $grouped->map(function ($group, $month) {
            return [
                'month' => $month,
                'total' => $group->sum('amount'),
                'average' => $group->avg('amount'),
                'count' => $group->count(),
            ];
        })->values();
        Cache::put("{$cachePrefix}_{$startDate}_{$endDate}", $summaries, now()->addMinutes(30));
        return $summaries;
    }

    public function generateSalesReport($startDate, $endDate)
    {
        return $this->buildPeriodSummary(Sale::class, $startDate, $endDate, 'sales');
    }

    public function generateReturnsReport($startDate, $endDate)
    {
        return $this->buildPeriodSummary(ReturnItem::class, $startDate, $endDate, 'returns');
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `compound_statement`, `expression_statement`, `assignment_expression`, `if_statement`, `return_statement`
- **Detection approach**: Sliding window comparison of statement sequences within and across method bodies. Compare normalized statement hashes in windows of 5+ statements. Flag windows with identical hash sequences.
- **S-expression query sketch**:
```scheme
(compound_statement
  (_) @stmt)

(method_declaration
  body: (compound_statement
    (_) @stmt))
```

### Pipeline Mapping
- **Pipeline name**: `duplicate_code`
- **Pattern name**: `repeated_logic_blocks`
- **Severity**: info
- **Confidence**: low
