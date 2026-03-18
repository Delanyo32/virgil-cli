# Encapsulation Leaks -- C++

## Overview
Encapsulation leaks in C++ occur when large objects are passed by value instead of `const` reference, causing unnecessary copies that degrade performance and expose internal representation through the copy, or when classes use `public` data members instead of private members with accessor methods, allowing any code to bypass validation and invariants. Both patterns undermine C++'s type system and encapsulation guarantees.

## Why It's a Tech Debt Concern
Passing large objects by value triggers copy constructors and potentially deep copies, wasting CPU and memory — especially problematic in hot loops or recursive calls. It also means the caller's object and the function's copy can diverge silently, creating confusion about which version holds the "correct" state. Public data members in classes (as opposed to structs used as plain data) defeat the purpose of having a class — any code can set fields to invalid combinations, and adding validation or change tracking later requires modifying every access site.

## Applicability
- **Relevance**: high (pass-by-value and public members are common in C++ code written by developers coming from C or Java)
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: Qt (large QObject-derived types), Boost, game engines (math types, entity components), Eigen (matrix types)

---

## Pattern 1: Large Object Passed by Value

### Description
A function accepts a large struct or class (containers like `std::vector`, `std::string`, `std::map`, or custom types with many fields) by value instead of by `const` reference. Each call copies the entire object, which is wasteful when the function only needs to read the data.

### Bad Code (Anti-pattern)
```cpp
struct SimulationState {
    std::vector<Particle> particles;      // potentially millions of elements
    std::map<int, Properties> properties;
    std::string description;
    Matrix4x4 transform;
    BoundingBox bounds;
    std::vector<Force> active_forces;
};

// Copies entire SimulationState on every call
double calculate_energy(SimulationState state) {
    double total = 0.0;
    for (const auto& p : state.particles) {
        total += 0.5 * p.mass * p.velocity.length_squared();
    }
    return total;
}

// Copies vector of strings
std::string join_names(std::vector<std::string> names, std::string separator) {
    std::string result;
    for (size_t i = 0; i < names.size(); ++i) {
        if (i > 0) result += separator;
        result += names[i];
    }
    return result;
}

// Copies map on every lookup call
bool has_property(std::map<std::string, Config> configs, std::string key) {
    return configs.find(key) != configs.end();
}

void render_frame(Scene scene, Camera camera, RenderOptions options) {
    // scene contains meshes, textures, lights — all copied
    for (const auto& mesh : scene.meshes) {
        draw(mesh, camera, options);
    }
}
```

### Good Code (Fix)
```cpp
struct SimulationState {
    std::vector<Particle> particles;
    std::map<int, Properties> properties;
    std::string description;
    Matrix4x4 transform;
    BoundingBox bounds;
    std::vector<Force> active_forces;
};

// const reference — no copy, read-only access
double calculate_energy(const SimulationState& state) {
    double total = 0.0;
    for (const auto& p : state.particles) {
        total += 0.5 * p.mass * p.velocity.length_squared();
    }
    return total;
}

// const references for read-only parameters
std::string join_names(const std::vector<std::string>& names, const std::string& separator) {
    std::string result;
    for (size_t i = 0; i < names.size(); ++i) {
        if (i > 0) result += separator;
        result += names[i];
    }
    return result;
}

bool has_property(const std::map<std::string, Config>& configs, const std::string& key) {
    return configs.find(key) != configs.end();
}

void render_frame(const Scene& scene, const Camera& camera, const RenderOptions& options) {
    for (const auto& mesh : scene.meshes) {
        draw(mesh, camera, options);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `parameter_declaration` inside `function_definition` where the type is a known large type and the declarator is not a `reference_declarator`
- **Detection approach**: Find `parameter_declaration` nodes in `function_definition` parameter lists where the type is a template type (`std::vector`, `std::map`, `std::string`, `std::unordered_map`, `std::set`) or a user-defined class/struct, and the declarator is a plain `identifier` (not a `reference_declarator` with `&`). Exclude parameters that are primitive types (`int`, `double`, `bool`, `char`, pointers). Flag each non-reference parameter of a container or class type.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    declarator: (function_declarator
      declarator: (_) @func_name
      parameters: (parameter_list
        (parameter_declaration
          type: (type_identifier) @param_type
          declarator: (identifier) @param_name))))

  (function_definition
    declarator: (function_declarator
      declarator: (_) @func_name
      parameters: (parameter_list
        (parameter_declaration
          type: (template_type
            name: (type_identifier) @template_name)
          declarator: (identifier) @param_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `large_object_by_value`
- **Pattern name**: `non_ref_class_parameter`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Public Data Members in Class

### Description
A class (not a plain struct) declares data members in the `public` section, allowing any code to directly read and modify internal state. This bypasses any invariant enforcement the class should provide and makes it impossible to add validation, caching, or change notification without modifying every access site.

### Bad Code (Anti-pattern)
```cpp
class BankAccount {
public:
    std::string owner_name;
    std::string account_number;
    double balance;
    double overdraft_limit;
    std::vector<Transaction> history;
    bool frozen;
    std::chrono::system_clock::time_point last_activity;

    BankAccount(const std::string& owner, const std::string& number)
        : owner_name(owner), account_number(number), balance(0.0),
          overdraft_limit(0.0), frozen(false),
          last_activity(std::chrono::system_clock::now()) {}

    void deposit(double amount) {
        balance += amount;
        history.push_back({TransactionType::Deposit, amount});
        last_activity = std::chrono::system_clock::now();
    }
};

// Any code can break invariants
BankAccount acct("Alice", "12345");
acct.balance = 1000000.0;    // bypass deposit logic
acct.balance = -999999.0;    // below overdraft limit
acct.frozen = false;          // unfreeze without authorization
acct.history.clear();         // destroy audit trail
acct.account_number = "99999"; // change identity
```

### Good Code (Fix)
```cpp
class BankAccount {
public:
    BankAccount(const std::string& owner, const std::string& number)
        : owner_name_(owner), account_number_(number), balance_(0.0),
          overdraft_limit_(0.0), frozen_(false),
          last_activity_(std::chrono::system_clock::now()) {}

    const std::string& owner_name() const { return owner_name_; }
    const std::string& account_number() const { return account_number_; }
    double balance() const { return balance_; }
    bool is_frozen() const { return frozen_; }
    const std::vector<Transaction>& history() const { return history_; }

    void deposit(double amount) {
        if (amount <= 0) throw std::invalid_argument("Amount must be positive");
        if (frozen_) throw std::runtime_error("Account is frozen");
        balance_ += amount;
        history_.push_back({TransactionType::Deposit, amount});
        last_activity_ = std::chrono::system_clock::now();
    }

    void withdraw(double amount) {
        if (amount <= 0) throw std::invalid_argument("Amount must be positive");
        if (frozen_) throw std::runtime_error("Account is frozen");
        if (balance_ - amount < -overdraft_limit_)
            throw std::runtime_error("Insufficient funds");
        balance_ -= amount;
        history_.push_back({TransactionType::Withdrawal, amount});
        last_activity_ = std::chrono::system_clock::now();
    }

    void freeze() { frozen_ = true; }

    void set_overdraft_limit(double limit) {
        if (limit < 0) throw std::invalid_argument("Limit must be non-negative");
        overdraft_limit_ = limit;
    }

private:
    std::string owner_name_;
    std::string account_number_;
    double balance_;
    double overdraft_limit_;
    std::vector<Transaction> history_;
    bool frozen_;
    std::chrono::system_clock::time_point last_activity_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_specifier` with `access_specifier` "public" followed by `field_declaration` nodes
- **Detection approach**: Find `class_specifier` nodes (not `struct_specifier` — structs conventionally have public data). Within the `field_declaration_list`, identify sections under a `public` `access_specifier`. Count `field_declaration` nodes (excluding `function_definition` and `declaration` of methods) in the public section. Flag classes with 2+ public non-static data members.
- **S-expression query sketch**:
  ```scheme
  (class_specifier
    name: (type_identifier) @class_name
    body: (field_declaration_list
      (access_specifier) @access
      (field_declaration
        type: (_) @field_type
        declarator: (field_identifier) @field_name)))
  ```

### Pipeline Mapping
- **Pipeline name**: `encapsulation_leaks`
- **Pattern name**: `public_class_data_members`
- **Severity**: warning
- **Confidence**: high
