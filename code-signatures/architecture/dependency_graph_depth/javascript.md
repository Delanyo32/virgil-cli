# Dependency Graph Depth -- JavaScript/TypeScript

## Overview
Dependency graph depth measures how many layers of module imports a file must traverse before reaching the actual implementation. In JavaScript and TypeScript, deep dependency chains are especially common due to the widespread use of barrel files (`index.ts`) that re-export from submodules, creating long transitive import chains that slow down bundlers, complicate tree-shaking, and make the codebase harder to navigate.

## Why It's an Architecture Concern
Deep dependency chains in JS/TS increase the blast radius of changes -- modifying a module buried several layers deep can trigger cascading re-exports through multiple barrel files, affecting consumers that never directly interact with the changed code. Barrel files create circular dependency risks, degrade bundler performance (especially with named re-exports that defeat tree-shaking), and add cognitive overhead when developers must trace imports through multiple `index.ts` files to find the actual source. Deeply nested relative paths (`../../../../`) signal excessive directory layering that makes the project structure brittle and hard to refactor.

## Applicability
- **Relevance**: high
- **Languages covered**: `.js, .jsx, .ts, .tsx`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In JavaScript/TypeScript, barrel files are `index.ts` (or `index.js`) files that import from submodules and re-export everything, providing a single entry point for a directory. While convenient for external package APIs, internal barrel files add unnecessary indirection layers, slow down module resolution, and can cause circular dependency issues.

### Bad Code (Anti-pattern)
```typescript
// src/components/index.ts -- barrel file re-exporting everything
export { Button } from './Button';
export { TextField } from './TextField';
export { Select } from './Select';
export { Checkbox } from './Checkbox';
export { RadioGroup } from './RadioGroup';
export { DatePicker } from './DatePicker';
export { Modal } from './Modal';
export { Tooltip } from './Tooltip';
export type { ButtonProps } from './Button';
export type { TextFieldProps } from './TextField';
```

### Good Code (Fix)
```typescript
// src/pages/Dashboard.tsx -- imports directly from source modules
import { Button } from '../components/Button';
import { TextField } from '../components/TextField';
import { Modal } from '../components/Modal';

export function Dashboard() {
  return (
    <Modal trigger={<Button label="Open" />}>
      <TextField placeholder="Search..." />
    </Modal>
  );
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `export_statement`
- **Detection approach**: Count `export { ... } from '...'` re-export statements in a single file. Flag if count >= 5, especially if the file is named `index.ts` or `index.js`. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Named re-exports
(export_statement
  (export_clause) @exports
  source: (string) @source_module) @reexport

;; Wildcard re-exports
(export_statement
  "*"
  source: (string) @source_module) @wildcard_reexport
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In JS/TS this appears as long relative paths with many `../` segments or scoped package imports with deep sub-paths.

### Bad Code (Anti-pattern)
```typescript
import { validateOrder } from '../../../../domain/orders/validation/rules';
import { OrderRepository } from '../../../../infrastructure/persistence/repositories/orders';
import { formatCurrency } from '../../../../shared/utils/formatting/currency';
import { logger } from '../../../../core/observability/logging/structured';
import type { OrderAggregate } from '@myorg/shared/domain/aggregates/orders/types';

export async function processOrder(id: string) {
  const order = await OrderRepository.findById(id);
  validateOrder(order);
  logger.info(`Processed ${formatCurrency(order.total)}`);
}
```

### Good Code (Fix)
```typescript
import { validateOrder } from '@/domain/orders';
import { OrderRepository } from '@/persistence/orders';
import { formatCurrency } from '@/utils/currency';
import { logger } from '@/logging';
import type { OrderAggregate } from '@myorg/shared/orders';

export async function processOrder(id: string) {
  const order = await OrderRepository.findById(id);
  validateOrder(order);
  logger.info(`Processed ${formatCurrency(order.total)}`);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`, `string` (module specifier)
- **Detection approach**: Parse the import path string. For relative paths, count `../` segments. For absolute/scoped paths, count `/`-separated segments after the package name. Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Static imports
(import_statement
  source: (string) @import_path) @import_stmt

;; Dynamic imports
(call_expression
  function: (import)
  arguments: (arguments
    (string) @dynamic_import_path)) @dynamic_import

;; Re-export sources (for chain analysis)
(export_statement
  source: (string) @reexport_path) @reexport
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
