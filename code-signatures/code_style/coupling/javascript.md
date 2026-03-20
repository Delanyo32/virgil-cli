# Coupling -- JavaScript/TypeScript

## Overview
Coupling measures how tightly interconnected modules, classes, or files are. High coupling means changes in one module cascade to many others. Low coupling with high cohesion is the goal of modular design.

## Why It's a Code Style Concern
Highly coupled code resists change — modifying one file requires updating many dependents. It makes unit testing difficult (many mocks needed), slows compilation in languages with explicit builds, and creates fragile architectures where small changes cause widespread breakage.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: N/A

---

## Pattern 1: Excessive Import Dependencies

### Description
A single file/module importing from many different modules (high fan-in), indicating it depends on too many parts of the system. Typically a "god module" that orchestrates everything. Barrel files (`index.ts`) that re-export everything amplify this problem by hiding transitive dependencies behind a single import path.

### Bad Code (Anti-pattern)
```typescript
// app/controllers/orderController.ts
import { authenticate, authorize } from '../auth/authService';
import { validateToken } from '../auth/tokenValidator';
import { getUserById } from '../users/userService';
import { getUserPreferences } from '../users/preferencesService';
import { createOrder, updateOrder } from '../orders/orderService';
import { validateOrder } from '../orders/orderValidator';
import { calculateTax } from '../billing/taxService';
import { processPayment } from '../billing/paymentGateway';
import { applyDiscount } from '../billing/discountEngine';
import { sendConfirmationEmail } from '../notifications/emailService';
import { sendPushNotification } from '../notifications/pushService';
import { logEvent } from '../logging/eventLogger';
import { trackAnalytics } from '../analytics/tracker';
import { cacheResult, invalidateCache } from '../cache/cacheManager';
import { enqueueJob } from '../queue/jobQueue';
import { formatCurrency } from '../utils/formatters';
import { db } from '../database/connection';
```

### Good Code (Fix)
```typescript
// app/controllers/orderController.ts
import { OrderService } from '../orders/orderService';
import { authenticate } from '../auth/authService';
import { logEvent } from '../logging/eventLogger';

export class OrderController {
  constructor(private orderService: OrderService) {}

  async createOrder(req: Request, res: Response) {
    const user = await authenticate(req);
    const order = await this.orderService.create(user, req.body);
    logEvent('order_created', { orderId: order.id });
    return res.json(order);
  }
}

// app/orders/orderService.ts — encapsulates billing, notifications, caching
import { BillingService } from '../billing/billingService';
import { NotificationService } from '../notifications/notificationService';
import { validateOrder } from './orderValidator';

export class OrderService {
  constructor(
    private billing: BillingService,
    private notifications: NotificationService,
  ) {}

  async create(user: User, data: OrderData) {
    validateOrder(data);
    const total = await this.billing.processOrder(data);
    await this.notifications.sendOrderConfirmation(user, total);
    return { ...data, total };
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement`, `call_expression` (for `require()`)
- **Detection approach**: Count unique import source strings per file. For ES modules, extract the `source` child (string node) of each `import_statement`. For CommonJS, find `require('...')` call expressions. Flag files exceeding threshold (e.g., 15+ unique module imports). Distinguish between standard library/node_modules imports and project-internal imports.
- **S-expression query sketch**:
```scheme
;; ES module imports
(import_statement
  source: (string) @import_source)

;; Dynamic imports
(call_expression
  function: (import)
  arguments: (arguments (string) @import_source))

;; CommonJS require
(call_expression
  function: (identifier) @func_name
  arguments: (arguments (string) @require_source)
  (#eq? @func_name "require"))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `excessive_imports`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Circular Dependencies

### Description
Two or more modules that import each other (directly or transitively), creating a dependency cycle. In JavaScript/TypeScript, circular imports can cause modules to receive `undefined` at import time, leading to subtle runtime errors. Barrel files (`index.ts`) frequently introduce hidden circular dependencies by re-exporting across module boundaries.

### Bad Code (Anti-pattern)
```typescript
// models/user.ts
import { Order } from './order';

export class User {
  orders: Order[] = [];
  addOrder(order: Order) {
    this.orders.push(order);
  }
}

// models/order.ts
import { User } from './user';  // Circular: order -> user -> order

export class Order {
  owner: User;
  constructor(user: User) {
    this.owner = user;
    user.addOrder(this);  // May fail at runtime — User could be undefined
  }
}
```

### Good Code (Fix)
```typescript
// models/types.ts — shared interface, no circular dependency
export interface IUser {
  id: string;
  addOrder(order: IOrder): void;
}

export interface IOrder {
  id: string;
  ownerId: string;
}

// models/user.ts
import { IUser, IOrder } from './types';

export class User implements IUser {
  id: string;
  orders: IOrder[] = [];
  addOrder(order: IOrder) {
    this.orders.push(order);
  }
}

// models/order.ts
import { IOrder } from './types';

export class Order implements IOrder {
  id: string;
  ownerId: string;
  constructor(ownerId: string, data: OrderData) {
    this.ownerId = ownerId;
    this.id = generateId();
  }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `import_statement` (source string), `call_expression` (require source)
- **Detection approach**: Build a directed graph of file-to-file imports by resolving relative import paths. Detect cycles using DFS with back-edge detection. Report the shortest cycle found. Pay special attention to barrel files (`index.ts`) which can mask transitive cycles.
- **S-expression query sketch**:
```scheme
;; Collect all import sources to build dependency graph
(import_statement
  source: (string) @import_path)

;; Also catch re-exports which participate in dependency chains
(export_statement
  source: (string) @reexport_path)

;; CommonJS require
(call_expression
  function: (identifier) @fn (#eq? @fn "require")
  arguments: (arguments (string) @require_path))
```

### Pipeline Mapping
- **Pipeline name**: `coupling`
- **Pattern name**: `circular_dependencies`
- **Severity**: warning
- **Confidence**: high
