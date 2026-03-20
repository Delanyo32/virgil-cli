# Circular Dependencies -- JavaScript/TypeScript

## Overview
Circular dependencies in JavaScript and TypeScript occur when two or more modules mutually `import` from each other, forming a cycle in the module graph. This is one of the most common architectural issues in JS/TS codebases. While Node.js and bundlers can often resolve circular imports at runtime, the imported bindings may be `undefined` at the point of use due to module evaluation order, leading to subtle and hard-to-diagnose runtime errors.

## Why It's an Architecture Concern
Circular imports make modules inseparable — you cannot extract, test, or deploy one without the other. They cause initialization ordering bugs where imported values are `undefined` because the importing module was evaluated before the exporting module finished initializing. Barrel files (`index.ts`) commonly amplify cycles by re-exporting modules that import back through the barrel. Bundlers like Webpack and Rollup may produce incorrect output or emit warnings when cycles are present. Cycles indicate tangled responsibilities: if module A needs types from B and B needs functions from A, neither module has a coherent, self-contained purpose.

## Applicability
- **Relevance**: high
- **Languages covered**: `.js, .jsx, .ts, .tsx`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```typescript
// --- models/user.ts ---
import { Order } from './order';  // user.ts imports order.ts

export interface User {
    id: number;
    name: string;
    orders: Order[];
}

export function getUserDisplayName(user: User): string {
    return `${user.name} (${user.orders.length} orders)`;
}

// --- models/order.ts ---
import { User } from './user';  // order.ts imports user.ts -- CIRCULAR

export interface Order {
    id: number;
    total: number;
    customer: User;  // references User
}

export function getOrderSummary(order: Order): string {
    return `Order #${order.id} for ${order.customer.name}: $${order.total}`;
}
```

### Good Code (Fix)
```typescript
// --- models/types.ts --- (shared types extracted to break cycle)
export interface User {
    id: number;
    name: string;
    orders: Order[];
}

export interface Order {
    id: number;
    total: number;
    customer: User;
}

// --- models/user.utils.ts ---
import { User } from './types';  // unidirectional

export function getUserDisplayName(user: User): string {
    return `${user.name} (${user.orders.length} orders)`;
}

// --- models/order.utils.ts ---
import { Order } from './types';  // unidirectional

export function getOrderSummary(order: Order): string {
    return `Order #${order.id} for ${order.customer.name}: $${order.total}`;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`
- **Detection approach**: Per-file: extract all import source paths from each module. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file to its imported modules, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files that both import from and are imported by the same module (requires cross-referencing imports.parquet).
- **S-expression query sketch**:
```scheme
(import_statement
  source: (string) @import_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `mutual_import`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hub Module (Bidirectional)

### Description
A module with high fan-in (many dependents) AND high fan-out (many dependencies), acting as a nexus that participates in or enables dependency cycles.

### Bad Code (Anti-pattern)
```typescript
// --- utils/index.ts --- (barrel file acting as a cycle nexus)
import { formatDate } from '../date/format';
import { validateEmail } from '../auth/validate';
import { parseConfig } from '../config/parser';
import { createLogger } from '../logging/logger';
import { cacheResult } from '../cache/store';
import { emitEvent } from '../events/emitter';
import { formatCurrency } from '../billing/format';

// Re-exports everything — high fan-out
export { formatDate, validateEmail, parseConfig,
         createLogger, cacheResult, emitEvent, formatCurrency };

// Every module above imports from utils/index.ts — high fan-in
// This creates implicit cycles through the barrel file
```

### Good Code (Fix)
```typescript
// --- date/format.ts --- (import directly, no barrel)
export function formatDate(d: Date): string {
    return d.toISOString().split('T')[0];
}

// --- auth/validate.ts ---
export function validateEmail(email: string): boolean {
    return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

// --- billing/format.ts ---
export function formatCurrency(amount: number): string {
    return `$${amount.toFixed(2)}`;
}

// Consumers import directly from the source module:
// import { formatDate } from '../date/format';
// import { validateEmail } from '../auth/validate';
// No barrel file hub needed
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`
- **Detection approach**: Per-file: count import statements to estimate fan-out. Also count `export` statements that re-export from other modules. Cross-file: query imports.parquet to count how many other files import from this file (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(import_statement
  source: (string) @import_source)

(export_statement
  source: (string) @reexport_source)
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
