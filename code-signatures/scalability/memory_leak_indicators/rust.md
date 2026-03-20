# Memory Leak Indicators -- Rust

## Overview
While Rust's ownership system prevents most memory leaks, they can still occur through `Box::leak()`, `std::mem::forget()`, reference-counting cycles (`Rc`/`Arc` with interior mutability), and unbounded collection growth in long-running applications.

## Why It's a Scalability Concern
Rust services are chosen for performance-critical paths. Memory leaks, though rarer than in GC'd languages, are particularly impactful because Rust programs often run as long-lived daemons with tight memory budgets. `Box::leak()` is permanent, `Rc` cycles are never collected, and unbounded `HashMap` growth in a service handling millions of requests will eventually exhaust available memory.

## Applicability
- **Relevance**: high
- **Languages covered**: .rs
- **Frameworks/libraries**: std (collections, sync), tokio, serde

---

## Pattern 1: HashMap/Vec Unbounded Growth in Loop

### Description
Calling `.insert()` on a `HashMap` or `.push()` on a `Vec` inside a loop or repeatedly-called function without any `.remove()`, `.retain()`, or capacity check.

### Bad Code (Anti-pattern)
```rust
use std::collections::HashMap;

static CACHE: Lazy<Mutex<HashMap<String, Vec<u8>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn handle_request(key: String, data: Vec<u8>) {
    let mut cache = CACHE.lock().unwrap();
    cache.insert(key, data);
    // never evicts — grows forever
}
```

### Good Code (Fix)
```rust
use std::collections::HashMap;
use lru::LruCache;
use std::num::NonZeroUsize;

static CACHE: Lazy<Mutex<LruCache<String, Vec<u8>>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(10000).unwrap())));

fn handle_request(key: String, data: Vec<u8>) {
    let mut cache = CACHE.lock().unwrap();
    cache.put(key, data);
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `field_expression`, `for_expression`, `loop_expression`
- **Detection approach**: Find `call_expression` where the function is `field_expression` with field `insert` (HashMap) or `push` (Vec) inside a `for_expression`, `loop_expression`, or `while_expression`. Also flag if the collection is in a `static` or module-level binding and no `.remove()`, `.retain()`, or `.clear()` exists.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (field_expression
    field: (field_identifier) @method)
  (#match? @method "^(insert|push)$"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `unbounded_collection_growth`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Box::leak() Usage

### Description
`Box::leak()` intentionally leaks memory by converting a `Box<T>` into a `&'static T`. While sometimes legitimate (e.g., for truly static data), it's often misused for convenience and the leaked memory is never reclaimed.

### Bad Code (Anti-pattern)
```rust
fn create_config(data: String) -> &'static str {
    Box::leak(data.into_boxed_str())
}

fn main() {
    for line in std::io::stdin().lock().lines() {
        let config = create_config(line.unwrap());
        process(config);
        // leaked string is never freed
    }
}
```

### Good Code (Fix)
```rust
fn create_config(data: String) -> String {
    data
}

fn main() {
    for line in std::io::stdin().lock().lines() {
        let config = create_config(line.unwrap());
        process(&config);
        // String is dropped at end of iteration
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`, `field_expression`
- **Detection approach**: Find `call_expression` where the function is `field_expression` with field `leak` on a `Box` type, or `scoped_identifier` `Box::leak`.
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier
    path: (identifier) @type
    name: (identifier) @method)
  (#eq? @type "Box")
  (#eq? @method "leak"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `box_leak_usage`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 3: Rc/Arc Cycle Patterns

### Description
Creating reference-counting cycles where `Rc<RefCell<T>>` or `Arc<Mutex<T>>` objects reference each other, preventing reference counts from reaching zero. Common with parent-child data structures.

### Bad Code (Anti-pattern)
```rust
use std::rc::Rc;
use std::cell::RefCell;

struct Node {
    children: Vec<Rc<RefCell<Node>>>,
    parent: Option<Rc<RefCell<Node>>>,
}

fn create_tree() -> Rc<RefCell<Node>> {
    let parent = Rc::new(RefCell::new(Node { children: vec![], parent: None }));
    let child = Rc::new(RefCell::new(Node { children: vec![], parent: Some(Rc::clone(&parent)) }));
    parent.borrow_mut().children.push(Rc::clone(&child));
    parent // cycle: parent -> child -> parent
}
```

### Good Code (Fix)
```rust
use std::rc::{Rc, Weak};
use std::cell::RefCell;

struct Node {
    children: Vec<Rc<RefCell<Node>>>,
    parent: Option<Weak<RefCell<Node>>>,
}

fn create_tree() -> Rc<RefCell<Node>> {
    let parent = Rc::new(RefCell::new(Node { children: vec![], parent: None }));
    let child = Rc::new(RefCell::new(Node { children: vec![], parent: Some(Rc::downgrade(&parent)) }));
    parent.borrow_mut().children.push(Rc::clone(&child));
    parent
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `struct_item`, `field_declaration`, `generic_type`, `type_identifier`
- **Detection approach**: Find `struct_item` where field declarations contain both `Rc<...>` (or `Arc<...>`) and the struct's own type name appears inside the generic parameter, indicating a potential self-referential cycle. Look for `Rc<RefCell<Self>>` or similar patterns.
- **S-expression query sketch**:
```scheme
(struct_item
  name: (type_identifier) @struct_name
  body: (field_declaration_list
    (field_declaration
      type: (generic_type
        type: (type_identifier) @wrapper
        type_arguments: (type_arguments
          (generic_type
            type_arguments: (type_arguments
              (type_identifier) @inner_type)))))))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `rc_arc_cycle`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 4: std::mem::forget() Usage

### Description
`std::mem::forget()` prevents a value's destructor from running, leaking any resources it holds (memory, file handles, locks). While safe, it's almost always a code smell.

### Bad Code (Anti-pattern)
```rust
fn transfer_ownership(data: Vec<u8>) -> *const u8 {
    let ptr = data.as_ptr();
    std::mem::forget(data); // Vec's memory is leaked
    ptr
}
```

### Good Code (Fix)
```rust
fn transfer_ownership(data: Vec<u8>) -> (*const u8, usize, usize) {
    let mut data = ManuallyDrop::new(data);
    (data.as_mut_ptr(), data.len(), data.capacity())
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `call_expression`, `scoped_identifier`
- **Detection approach**: Find `call_expression` calling `std::mem::forget` or `mem::forget` (when `use std::mem` is in scope).
- **S-expression query sketch**:
```scheme
(call_expression
  function: (scoped_identifier) @func_path
  (#match? @func_path "mem.*forget"))
```

### Pipeline Mapping
- **Pipeline name**: `memory_leak_indicators`
- **Pattern name**: `mem_forget_usage`
- **Severity**: warning
- **Confidence**: high
