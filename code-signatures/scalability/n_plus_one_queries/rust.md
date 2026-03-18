# N+1 Queries -- Rust

## Overview
N+1 query patterns in Rust occur when database query functions from crates like sqlx, diesel, or sea-orm are called inside `for`, `while`, or `loop` constructs instead of using batch queries with `IN` clauses or joins.

## Why It's a Scalability Concern
Rust's async runtimes (tokio, async-std) are designed for high concurrency, but N+1 patterns serialize database calls within each request. Each `.await` in a loop yields the task, and the next query can only start after the previous completes. This negates the concurrency benefits and creates per-request latency that scales with N.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: sqlx, diesel, sea-orm, tokio-postgres, reqwest

---

## Pattern 1: sqlx Query in Loop

### Description
Calling `sqlx::query()`, `.fetch_one()`, `.fetch_optional()`, or `.execute()` inside a `for`, `while`, or `loop` block.

### Bad Code (Anti-pattern)
```rust
async fn get_order_details(pool: &PgPool, order_ids: &[i64]) -> Result<Vec<OrderDetail>> {
    let mut results = Vec::new();
    for id in order_ids {
        let order = sqlx::query_as!(Order, "SELECT * FROM orders WHERE id = $1", id)
            .fetch_one(pool)
            .await?;
        results.push(OrderDetail { order });
    }
    Ok(results)
}
```

### Good Code (Fix)
```rust
async fn get_order_details(pool: &PgPool, order_ids: &[i64]) -> Result<Vec<OrderDetail>> {
    let orders = sqlx::query_as!(Order, "SELECT * FROM orders WHERE id = ANY($1)", order_ids)
        .fetch_all(pool)
        .await?;
    Ok(orders.into_iter().map(|order| OrderDetail { order }).collect())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_expression`, `while_expression`, `loop_expression`, `call_expression`, `field_expression`, `await_expression`
- **Detection approach**: Find `call_expression` or macro invocation (`macro_invocation`) for `sqlx::query`, `sqlx::query_as` followed by method chains (`.fetch_one`, `.execute`), nested inside a loop. The `.await` expression wraps the chain.
- **S-expression query sketch**:
```scheme
(for_expression
  body: (block
    (let_declaration
      value: (await_expression
        (call_expression
          function: (field_expression
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `sqlx_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Diesel Query in Loop

### Description
Executing diesel query builder calls like `.filter().first()` or `.load()` inside a loop, issuing individual queries instead of using `.filter(column.eq_any(&ids))`.

### Bad Code (Anti-pattern)
```rust
fn get_users(conn: &mut PgConnection, user_ids: &[i32]) -> QueryResult<Vec<UserWithPosts>> {
    let mut results = Vec::new();
    for id in user_ids {
        let user = users::table.find(id).first::<User>(conn)?;
        let user_posts = posts::table.filter(posts::user_id.eq(id)).load::<Post>(conn)?;
        results.push(UserWithPosts { user, posts: user_posts });
    }
    Ok(results)
}
```

### Good Code (Fix)
```rust
fn get_users(conn: &mut PgConnection, user_ids: &[i32]) -> QueryResult<Vec<UserWithPosts>> {
    let all_users = users::table
        .filter(users::id.eq_any(user_ids))
        .load::<User>(conn)?;
    let all_posts = posts::table
        .filter(posts::user_id.eq_any(user_ids))
        .load::<Post>(conn)?;
    let posts_by_user: HashMap<i32, Vec<Post>> = all_posts
        .into_iter()
        .into_group_map_by(|p| p.user_id);
    Ok(all_users.into_iter().map(|user| UserWithPosts {
        posts: posts_by_user.get(&user.id).cloned().unwrap_or_default(),
        user,
    }).collect())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_expression`, `call_expression`, `field_expression`
- **Detection approach**: Find method call chains ending in `.first()`, `.load()`, `.get_result()` on diesel table references inside a `for_expression`.
- **S-expression query sketch**:
```scheme
(for_expression
  body: (block
    (let_declaration
      value: (try_expression
        (call_expression
          function: (field_expression
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `diesel_query_in_loop`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: HTTP Client Call in Loop

### Description
Calling `reqwest::get()`, `client.get().send()`, or similar HTTP client methods inside a loop, making sequential network requests.

### Bad Code (Anti-pattern)
```rust
async fn enrich_users(client: &reqwest::Client, users: &mut [User]) -> Result<()> {
    for user in users.iter_mut() {
        let profile: Profile = client
            .get(&format!("https://api.example.com/profiles/{}", user.id))
            .send()
            .await?
            .json()
            .await?;
        user.profile = Some(profile);
    }
    Ok(())
}
```

### Good Code (Fix)
```rust
async fn enrich_users(client: &reqwest::Client, users: &mut [User]) -> Result<()> {
    let ids: Vec<String> = users.iter().map(|u| u.id.to_string()).collect();
    let profiles: Vec<Profile> = client
        .get("https://api.example.com/profiles")
        .query(&[("ids", ids.join(","))])
        .send()
        .await?
        .json()
        .await?;
    let profile_map: HashMap<i64, Profile> = profiles.into_iter().map(|p| (p.user_id, p)).collect();
    for user in users.iter_mut() {
        user.profile = profile_map.remove(&user.id);
    }
    Ok(())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `for_expression`, `call_expression`, `field_expression`, `await_expression`
- **Detection approach**: Find `call_expression` invoking `.get()`, `.post()`, `.send()` on a client-like variable or `reqwest::get` inside a `for_expression`.
- **S-expression query sketch**:
```scheme
(for_expression
  body: (block
    (let_declaration
      value: (await_expression
        (call_expression
          function: (field_expression
            field: (field_identifier) @method))))))
```

### Pipeline Mapping
- **Pipeline name**: `n_plus_one_queries`
- **Pattern name**: `http_call_in_loop`
- **Severity**: warning
- **Confidence**: medium
