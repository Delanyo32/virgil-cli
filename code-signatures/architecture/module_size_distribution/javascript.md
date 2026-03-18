# Module Size Distribution -- JavaScript/TypeScript

## Overview
Module size distribution measures how symbol definitions are spread across source files in a JavaScript or TypeScript codebase. Balanced module sizes make the codebase easier to navigate, improve tree-shaking effectiveness, and keep code reviews focused. Modules that are too large become difficult to maintain, while modules that are too small fragment logic across too many files.

## Why It's an Architecture Concern
Oversized modules concentrate too many functions, classes, and type definitions into a single file, making it difficult to reason about the module's purpose, increasing merge conflict frequency, and bloating bundle sizes when consumers import even one symbol from the file. They also tend to accumulate circular dependency edges as internal symbols reference each other. Anemic modules that export a single trivial symbol create unnecessary import chains and file system clutter, forcing developers to navigate many files to follow even simple code paths.

## Applicability
- **Relevance**: high
- **Languages covered**: `.js, .jsx, .ts, .tsx`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```typescript
// utils.ts -- a grab-bag of unrelated utilities
export function formatDate(d: Date): string { /* ... */ }
export function parseDate(s: string): Date { /* ... */ }
export function formatCurrency(amount: number): string { /* ... */ }
export function slugify(s: string): string { /* ... */ }
export function debounce<T extends Function>(fn: T, ms: number): T { /* ... */ }
export function throttle<T extends Function>(fn: T, ms: number): T { /* ... */ }
export function deepClone<T>(obj: T): T { /* ... */ }
export function mergeObjects<T>(a: T, b: Partial<T>): T { /* ... */ }

export type Nullable<T> = T | null;
export type DeepPartial<T> = { [K in keyof T]?: DeepPartial<T[K]> };

export interface Logger { log(msg: string): void; }
export interface Cache<T> { get(key: string): T | undefined; }

export class EventEmitter { /* ... */ }
export class HttpClient { /* ... */ }

export const MAX_RETRIES = 5;
export const DEFAULT_TIMEOUT = 30000;
// ... 15 more exported functions, types, and constants
```

### Good Code (Fix)
```typescript
// format.ts -- focused on formatting
export function formatDate(d: Date): string { /* ... */ }
export function parseDate(s: string): Date { /* ... */ }
export function formatCurrency(amount: number): string { /* ... */ }
```

```typescript
// async-utils.ts -- focused on async helpers
export function debounce<T extends Function>(fn: T, ms: number): T { /* ... */ }
export function throttle<T extends Function>(fn: T, ms: number): T { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `class_declaration`, `lexical_declaration`, `variable_declaration`, `type_alias_declaration`, `interface_declaration`, `enum_declaration`, `export_statement`
- **Detection approach**: Count all top-level symbol definitions (direct children of `program`). For `lexical_declaration` and `variable_declaration`, count each declarator. Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(program
  [
    (function_declaration name: (identifier) @name) @def
    (class_declaration name: (type_identifier) @name) @def
    (lexical_declaration (variable_declarator name: (identifier) @name)) @def
    (type_alias_declaration name: (type_identifier) @name) @def
    (interface_declaration name: (type_identifier) @name) @def
    (enum_declaration name: (identifier) @name) @def
    (export_statement
      declaration: [
        (function_declaration name: (identifier) @name)
        (class_declaration name: (type_identifier) @name)
        (lexical_declaration (variable_declarator name: (identifier) @name))
        (type_alias_declaration name: (type_identifier) @name)
        (interface_declaration name: (type_identifier) @name)
        (enum_declaration name: (identifier) @name)
      ]) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```typescript
// index.ts -- barrel file re-exporting everything
export { formatDate, parseDate, formatCurrency } from './format';
export { debounce, throttle } from './async-utils';
export { deepClone, mergeObjects } from './objects';
export { HttpClient } from './http';
export { EventEmitter } from './events';
export { Logger, Cache } from './interfaces';
export { MAX_RETRIES, DEFAULT_TIMEOUT } from './constants';
export type { Nullable, DeepPartial } from './types';
export { validateEmail, validateUrl } from './validation';
export { slugify, capitalize, truncate } from './strings';
// 25+ total re-exports
```

### Good Code (Fix)
```typescript
// format/index.ts -- focused sub-module
export { formatDate, parseDate } from './date';
export { formatCurrency } from './currency';
```

```typescript
// async/index.ts -- focused sub-module
export { debounce, throttle } from './timing';
```

### Tree-sitter Detection Strategy
- **Target node types**: `export_statement`, `export_specifier`
- **Detection approach**: Count all exported symbols -- both direct `export` declarations and `export { ... }` specifiers. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(export_statement
  declaration: (_) @decl) @export

(export_statement
  (export_clause
    (export_specifier name: (identifier) @name))) @export
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```typescript
// maxRetries.ts
export const MAX_RETRIES = 5;
```

### Good Code (Fix)
```typescript
// config.ts -- merge the trivial constant into a related module
export const MAX_RETRIES = 5;
export const DEFAULT_TIMEOUT = 30000;
export const API_VERSION = 'v2';

export function loadConfig(path: string): Config { /* ... */ }
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_declaration`, `class_declaration`, `lexical_declaration`, `variable_declaration`, `type_alias_declaration`, `interface_declaration`, `enum_declaration`, `export_statement`
- **Detection approach**: Count top-level symbol definitions (direct children of `program`). Flag if count == 1, excluding test files (`*.test.ts`, `*.spec.ts`), entry points (`index.ts`, `main.ts`), and type-only declaration files.
- **S-expression query sketch**:
```scheme
(program
  [
    (function_declaration name: (identifier) @name) @def
    (class_declaration name: (type_identifier) @name) @def
    (lexical_declaration (variable_declarator name: (identifier) @name)) @def
    (type_alias_declaration name: (type_identifier) @name) @def
    (interface_declaration name: (type_identifier) @name) @def
    (enum_declaration name: (identifier) @name) @def
    (export_statement
      declaration: [
        (function_declaration name: (identifier) @name)
        (class_declaration name: (type_identifier) @name)
        (lexical_declaration (variable_declarator name: (identifier) @name))
        (type_alias_declaration name: (type_identifier) @name)
        (interface_declaration name: (type_identifier) @name)
        (enum_declaration name: (identifier) @name)
      ]) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
