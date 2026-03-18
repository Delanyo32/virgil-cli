# N+1 Queries -- JavaScript

## Overview
N+1 query patterns occur when code executes a database query or API call inside a loop, resulting in one initial query plus N additional queries for each result. This is one of the most common performance anti-patterns in JavaScript/TypeScript web applications.

## Why It's a Scalability Concern
Each iteration triggers a separate round-trip to the database or external service, causing latency to scale linearly with dataset size. A page that loads 10 items makes 11 queries; at 1000 items it makes 1001 queries. This overwhelms connection pools, saturates database resources, and creates cascading latency under load.

## Applicability
- **Relevance**: high
- **Languages covered**: .js, .jsx, .ts, .tsx
- **Frameworks/libraries**: Prisma, Sequelize, Mongoose, TypeORM, Knex, node-postgres, fetch API

---

## Pattern 1: ORM findOne/findUnique Inside Loop

### Description
Calling ORM lookup methods like `Model.findOne()`, `Model.findById()`, or `prisma.*.findUnique()` inside a `for`, `for...of`, or `while` loop.

### Bad Code (Anti-pattern)
```typescript
async function getOrderDetails(orderIds: string[]) {
  const results = [];
  for (const id of orderIds) {
    const order = await prisma.order.findUnique({ where: { id } });
    const customer = await prisma.customer.findUnique({ where: { id: order.customerId } });
    results.push({ order, customer });
  }
  return results;
}
```

### Good Code (Fix)
```typescript
async function getOrderDetails(orderIds: string[]) {
  const orders = await prisma.order.findMany({
    where: { id: { in: orderIds } },
    include: { customer: true },
  });
  return orders.map(order => ({ order, customer: order.customer }));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_statement`, `for_in_statement`, `while_statement`, `call_expression`, `await_expression`, `member_expression`
- **Detection approach**: Find `call_expression` nodes whose callee is a `member_expression` ending in `findOne`, `findUnique`, `findById`, `findFirst` that are nested inside a loop construct (`for_statement`, `for_in_statement`, `while_statement`). Walk ancestors of the call to confirm loop containment.
- **S-expression query sketch**:
```scheme
(for_in_statement
  body: (_
    (expression_statement
      (await_expression
        (call_expression
          function: (member_expression
            property: (property_identifier) @method))))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `orm_find_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mongoose Query Inside Loop

### Description
Calling Mongoose methods like `.find()`, `.findById()`, `.findOne()` inside a loop, typically when populating related documents.

### Bad Code (Anti-pattern)
```javascript
async function getUsersWithPosts(userIds) {
  const results = [];
  for (const userId of userIds) {
    const user = await User.findById(userId);
    const posts = await Post.find({ author: userId });
    results.push({ user, posts });
  }
  return results;
}
```

### Good Code (Fix)
```javascript
async function getUsersWithPosts(userIds) {
  const users = await User.find({ _id: { $in: userIds } });
  const posts = await Post.find({ author: { $in: userIds } });
  const postsByUser = posts.reduce((map, post) => {
    (map[post.author] ||= []).push(post);
    return map;
  }, {});
  return users.map(user => ({ user, posts: postsByUser[user._id] || [] }));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_in_statement`, `call_expression`, `member_expression`, `await_expression`
- **Detection approach**: Match `call_expression` where the callee's property is `findById`, `find`, or `findOne` on a capitalized identifier (model name convention), nested within a loop body.
- **S-expression query sketch**:
```scheme
(for_in_statement
  body: (_
    (expression_statement
      (await_expression
        (call_expression
          function: (member_expression
            object: (identifier) @model
            property: (property_identifier) @method))))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `mongoose_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Fetch/HTTP Call Inside Loop

### Description
Making HTTP requests via `fetch()`, `axios.get()`, or similar inside a loop, causing N sequential network round-trips.

### Bad Code (Anti-pattern)
```typescript
async function enrichUsers(users: User[]) {
  for (const user of users) {
    const profile = await fetch(`/api/profiles/${user.id}`);
    user.profile = await profile.json();
  }
  return users;
}
```

### Good Code (Fix)
```typescript
async function enrichUsers(users: User[]) {
  const profiles = await fetch(`/api/profiles?ids=${users.map(u => u.id).join(',')}`);
  const profileData = await profiles.json();
  const profileMap = new Map(profileData.map((p: Profile) => [p.userId, p]));
  return users.map(user => ({ ...user, profile: profileMap.get(user.id) }));
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_in_statement`, `for_statement`, `while_statement`, `call_expression`, `await_expression`
- **Detection approach**: Find `call_expression` nodes calling `fetch`, `axios.get`, `axios.post`, `http.get` that are nested inside any loop construct. The call may be wrapped in `await_expression`.
- **S-expression query sketch**:
```scheme
(for_in_statement
  body: (_
    (expression_statement
      (await_expression
        (call_expression
          function: (identifier) @func_name)))) @loop)
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `http_call_in_loop`
- **Severity**: warning
- **Confidence**: medium
