# API Surface Area -- JavaScript

## Overview
API surface area in JavaScript and TypeScript is determined by `export` statements. Any function, class, variable, or type marked with `export` becomes part of the module's public contract. Since JS/TS modules can export an arbitrary number of symbols, it is easy for files to grow into "barrel" modules with massive export lists. Tracking the exported-to-total ratio identifies files that lack internal encapsulation, increasing the coupling surface for consumers.

## Why It's an Architecture Concern
Every exported symbol is a dependency point: other modules import it by name, and renaming or removing it becomes a breaking change for the dependency graph. A file that exports nearly everything provides no abstraction boundary — consumers couple to implementation details rather than a curated interface. In TypeScript especially, exported types and interfaces propagate through the type system, and excessive exports create wide, shallow APIs that are difficult to version and document. Keeping exports minimal by prefixing internal helpers (or simply not exporting them) allows modules to evolve their internals freely.

## Applicability
- **Relevance**: medium
- **Languages covered**: `.js, .jsx, .ts, .tsx`
- **Frameworks/libraries**: general

---

## Pattern 1: Excessive Public API

### Description
File where more than 80% of 10 or more symbols are exported, indicating minimal encapsulation and a wide coupling surface.

### Bad Code (Anti-pattern)
```typescript
export function fetchUser(id: string): Promise<User> { /* ... */ }
export function fetchUsers(): Promise<User[]> { /* ... */ }
export function createUser(data: UserInput): Promise<User> { /* ... */ }
export function updateUser(id: string, data: Partial<UserInput>): Promise<User> { /* ... */ }
export function deleteUser(id: string): Promise<void> { /* ... */ }
export function validateUserInput(data: UserInput): boolean { return true; }
export function hashPassword(pw: string): string { return ""; }
export function comparePassword(pw: string, hash: string): boolean { return true; }
export function formatUserResponse(user: User): UserDTO { return {} as UserDTO; }
export function buildUserQuery(filters: Filters): string { return ""; }
export function parseUserRow(row: any): User { return {} as User; }
export interface UserInput { name: string; email: string; }
```

### Good Code (Fix)
```typescript
// Public API — what consumers need
export function fetchUser(id: string): Promise<User> { /* ... */ }
export function fetchUsers(): Promise<User[]> { /* ... */ }
export function createUser(data: UserInput): Promise<User> { /* ... */ }
export function updateUser(id: string, data: Partial<UserInput>): Promise<User> { /* ... */ }
export function deleteUser(id: string): Promise<void> { /* ... */ }
export interface UserInput { name: string; email: string; }

// Internal helpers — not exported
function validateUserInput(data: UserInput): boolean { return true; }
function hashPassword(pw: string): string { return ""; }
function comparePassword(pw: string, hash: string): boolean { return true; }
function formatUserResponse(user: User): UserDTO { return {} as UserDTO; }
function buildUserQuery(filters: Filters): string { return ""; }
function parseUserRow(row: any): User { return {} as User; }
```

### Tree-sitter Detection Strategy
- **Target node types**: `export_statement`, `function_declaration`, `class_declaration`, `lexical_declaration`, `variable_declaration`, `interface_declaration`, `type_alias_declaration`
- **Detection approach**: Count all top-level declarations (functions, classes, variables, interfaces, type aliases). Count those wrapped in `export_statement`. Flag files where total >= 10 and exported/total > 0.8.
- **S-expression query sketch**:
```scheme
;; Match exported declarations
(export_statement
  declaration: [
    (function_declaration name: (identifier) @exported.name)
    (class_declaration name: (type_identifier) @exported.name)
    (lexical_declaration
      (variable_declarator name: (identifier) @exported.name))
    (interface_declaration name: (type_identifier) @exported.name)
    (type_alias_declaration name: (type_identifier) @exported.name)
  ])

;; Match all top-level declarations (exported or not)
(program
  [
    (function_declaration name: (identifier) @all.name)
    (class_declaration name: (type_identifier) @all.name)
    (lexical_declaration
      (variable_declarator name: (identifier) @all.name))
    (export_statement)
  ])
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `excessive_public_api`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Leaky Abstraction Boundary

### Description
Exported types expose implementation details such as public fields, mutable collections, or concrete types instead of interfaces/traits.

### Bad Code (Anti-pattern)
```typescript
export class CacheManager {
    public store: Map<string, Buffer> = new Map();
    public keys: string[] = [];
    public hitCount: number = 0;
    public missCount: number = 0;
    public maxSize: number;
    public evictionTimer: NodeJS.Timeout | null = null;

    get(key: string): Buffer | undefined { return this.store.get(key); }
    set(key: string, value: Buffer): void { this.store.set(key, value); }
}
```

### Good Code (Fix)
```typescript
export interface Cache {
    get(key: string): Buffer | undefined;
    set(key: string, value: Buffer): void;
    stats(): { hits: number; misses: number };
}

export function createCache(maxSize: number): Cache {
    const store = new Map<string, Buffer>();
    let hitCount = 0;
    let missCount = 0;

    return {
        get(key) { /* ... */ return undefined; },
        set(key, value) { /* ... */ },
        stats() { return { hits: hitCount, misses: missCount }; },
    };
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `public_field_definition`, `property_definition` inside exported `class_declaration`
- **Detection approach**: Find exported classes and inspect their field definitions. Fields with `public` modifier (or no modifier in JS, which defaults to public) that use concrete types like `Map`, `Array`, or `Set` indicate leaked internals. Flag classes with 3+ public data fields.
- **S-expression query sketch**:
```scheme
;; Match public fields in exported classes
(export_statement
  declaration: (class_declaration
    name: (type_identifier) @class.name
    body: (class_body
      (public_field_definition
        name: (property_identifier) @field.name
        type: (type_annotation) @field.type))))

;; Match fields without access modifier (default public in JS)
(export_statement
  declaration: (class_declaration
    body: (class_body
      (field_definition
        property: (property_identifier) @field.name))))
```

### Pipeline Mapping
- **Pipeline name**: `api_surface_area`
- **Pattern name**: `leaky_abstraction_boundary`
- **Severity**: warning
- **Confidence**: medium

---
