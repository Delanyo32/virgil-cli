# False Positive Injection Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite 10 overly-broad injection audit pipelines to taint-flow analysis and fix 2 over-broad Rust pipelines with `#match?` predicates, eliminating thousands of false positives while preserving all true positive detections.

**Architecture:** Pure JSON changes — no Rust code changes. Each injection pipeline is rewritten from a broad `match_pattern` stage to `taint_sources` → `taint_sanitizers` → `taint_sinks` → `flag`, following `sql_injection_python.json`. Tests are updated: positive tests get fixtures with auto-tainted parameters (names in `["request","req","input","body","query","params","args","argv","data","payload","form","user_input","raw_input","stdin"]` are auto-tainted by the engine), and an anti-FP test demonstrates the current false positive before each fix.

**Tech Stack:** JSON audit pipeline DSL, Rust integration tests (`tests/audit_json_integration.rs`), `cargo test`

**Key CFG facts** (needed to write correct fixtures):
- Python: call name = full `function` text (e.g., `eval`, `os.system`, `subprocess.run`)
- Go: call name = full selector text (e.g., `exec.Command`)
- Java: call name = `name` field only (e.g., `exec` from `runtime.exec(...)`)
- C/C++: call name = full `function` text (e.g., `system`, `popen`)
- Sink matching: substring match on call name (no trailing `(` needed in patterns)
- Taint source matching: substring match on source_vars (identifiers in RHS expressions)

---

### Task 1: Python code_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/code_injection_python.json`
- Modify: `tests/audit_json_integration.rs` (functions: `code_injection_python_finds_direct_call`, `code_injection_python_clean`)

- [ ] **Step 1: Add anti-FP test that currently fails**

In `tests/audit_json_integration.rs`, add after `code_injection_python_clean`:

```rust
#[test]
fn code_injection_python_no_fp_eval_literal() {
    let dir = tempfile::tempdir().unwrap();
    // eval() with a literal arg — no taint source anywhere in the file.
    // Currently fires (broad match_pattern); after fix must NOT fire.
    std::fs::write(dir.path().join("test.py"), "eval(\"1 + 1\")\n").unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "code_injection"),
        "expected no code_injection finding for eval with literal; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update the positive test to use an auto-tainted parameter**

Replace `code_injection_python_finds_direct_call` with:

```rust
#[test]
fn code_injection_python_finds_direct_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'request' is auto-tainted (matches PARAM_PATTERNS); eval() is the sink.
    std::fs::write(
        dir.path().join("test.py"),
        "def handle(request):\n    user_input = request.args.get('cmd')\n    eval(user_input)\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "code_injection" && f.pattern == "code_injection_call"),
        "expected code_injection/code_injection_call finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test code_injection_python -- --nocapture 2>&1 | tail -20
```

Expected: `code_injection_python_no_fp_eval_literal` FAILS ("expected no finding" assertion fails because current broad pipeline fires on `eval(...)`). Other tests pass.

- [ ] **Step 4: Rewrite `src/audit/builtin/code_injection_python.json`**

```json
{
  "pipeline": "code_injection",
  "category": "security",
  "description": "Detect code injection in Python via taint analysis: user-controlled data reaching eval(), exec(), or compile() sinks",
  "languages": ["python"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "request.form", "kind": "user_input"},
        {"pattern": "request.args", "kind": "user_input"},
        {"pattern": "request.data", "kind": "user_input"},
        {"pattern": "request.json", "kind": "user_input"},
        {"pattern": "request.values", "kind": "user_input"},
        {"pattern": "request.POST", "kind": "user_input"},
        {"pattern": "request.GET", "kind": "user_input"},
        {"pattern": "request.body", "kind": "user_input"},
        {"pattern": "input(", "kind": "user_input"},
        {"pattern": "sys.argv", "kind": "user_input"},
        {"pattern": "os.environ", "kind": "env_var"},
        {"pattern": "os.environ.get", "kind": "env_var"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "eval", "vulnerability": "code_injection"},
        {"pattern": "exec", "vulnerability": "code_injection"},
        {"pattern": "compile", "vulnerability": "code_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "code_injection_call",
        "message": "Code injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all Python code_injection tests**

```
cargo test code_injection_python -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass (`finds_direct_call`, `clean`, `no_fp_eval_literal`).

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/code_injection_python.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite code_injection_python as taint-flow pipeline"
```

---

### Task 2: Python command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_python.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_python_finds_attribute_call`, `command_injection_python_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_python_clean`:

```rust
#[test]
fn command_injection_python_no_fp_os_path_literal() {
    let dir = tempfile::tempdir().unwrap();
    // os.system() with a literal command — no taint source anywhere.
    // Currently fires (all attribute calls match); after fix must NOT fire.
    std::fs::write(dir.path().join("test.py"), "import os\nos.system(\"ls -la\")\n").unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection"),
        "expected no command_injection finding for os.system with literal; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test with auto-tainted parameter**

Replace `command_injection_python_finds_attribute_call` with:

```rust
#[test]
fn command_injection_python_finds_attribute_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'request' is auto-tainted; os.system() is the sink.
    std::fs::write(
        dir.path().join("test.py"),
        "import os\ndef run(request):\n    cmd = request.args.get('cmd')\n    os.system(cmd)\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Python], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Python]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Python])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for Python; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_python -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_python_no_fp_os_path_literal` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_python.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in Python via taint analysis: user-controlled data reaching os.system, os.popen, or subprocess sinks",
  "languages": ["python"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "request.form", "kind": "user_input"},
        {"pattern": "request.args", "kind": "user_input"},
        {"pattern": "request.data", "kind": "user_input"},
        {"pattern": "request.json", "kind": "user_input"},
        {"pattern": "request.values", "kind": "user_input"},
        {"pattern": "request.POST", "kind": "user_input"},
        {"pattern": "request.GET", "kind": "user_input"},
        {"pattern": "request.body", "kind": "user_input"},
        {"pattern": "input(", "kind": "user_input"},
        {"pattern": "sys.argv", "kind": "user_input"},
        {"pattern": "os.environ", "kind": "env_var"},
        {"pattern": "os.environ.get", "kind": "env_var"}
      ]
    },
    {
      "taint_sanitizers": [
        {"pattern": "shlex.quote"}
      ]
    },
    {
      "taint_sinks": [
        {"pattern": "os.system", "vulnerability": "command_injection"},
        {"pattern": "os.popen", "vulnerability": "command_injection"},
        {"pattern": "subprocess.run", "vulnerability": "command_injection"},
        {"pattern": "subprocess.call", "vulnerability": "command_injection"},
        {"pattern": "subprocess.Popen", "vulnerability": "command_injection"},
        {"pattern": "subprocess.check_output", "vulnerability": "command_injection"},
        {"pattern": "subprocess.check_call", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all Python command_injection tests**

```
cargo test command_injection_python -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_python.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_python as taint-flow pipeline"
```

---

### Task 3: JavaScript code_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/code_injection_javascript.json`
- Modify: `tests/audit_json_integration.rs` (functions: `code_injection_javascript_finds_direct_call`, `code_injection_javascript_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `code_injection_javascript_clean`:

```rust
#[test]
fn code_injection_javascript_no_fp_eval_literal() {
    let dir = tempfile::tempdir().unwrap();
    // eval() with a string literal — no taint source. Must NOT fire after fix.
    std::fs::write(dir.path().join("test.js"), "eval(\"1 + 1\");\n").unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "code_injection"),
        "expected no code_injection finding for eval with literal; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test**

Replace `code_injection_javascript_finds_direct_call` with:

```rust
#[test]
fn code_injection_javascript_finds_direct_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'req' is auto-tainted; eval() is the sink.
    std::fs::write(
        dir.path().join("test.js"),
        "function handle(req, res) {\n  const code = req.query.code;\n  eval(code);\n}\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "code_injection" && f.pattern == "code_injection_call"),
        "expected code_injection/code_injection_call finding for JavaScript; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test code_injection_javascript -- --nocapture 2>&1 | tail -20
```

Expected: `code_injection_javascript_no_fp_eval_literal` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/code_injection_javascript.json`**

```json
{
  "pipeline": "code_injection",
  "category": "security",
  "description": "Detect code injection in JavaScript/TypeScript via taint analysis: user-controlled data reaching eval(), Function(), or vm.run* sinks",
  "languages": ["javascript", "jsx", "typescript", "tsx"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "req.body", "kind": "user_input"},
        {"pattern": "req.query", "kind": "user_input"},
        {"pattern": "req.params", "kind": "user_input"},
        {"pattern": "req.headers", "kind": "user_input"},
        {"pattern": "request.body", "kind": "user_input"},
        {"pattern": "request.query", "kind": "user_input"},
        {"pattern": "process.env", "kind": "env_var"},
        {"pattern": "process.argv", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": [
        {"pattern": "escape"},
        {"pattern": "encodeURIComponent"}
      ]
    },
    {
      "taint_sinks": [
        {"pattern": "eval", "vulnerability": "code_injection"},
        {"pattern": "Function", "vulnerability": "code_injection"},
        {"pattern": "vm.runInContext", "vulnerability": "code_injection"},
        {"pattern": "vm.runInNewContext", "vulnerability": "code_injection"},
        {"pattern": "vm.runInThisContext", "vulnerability": "code_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "code_injection_call",
        "message": "Code injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all JavaScript code_injection tests**

```
cargo test code_injection_javascript -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/code_injection_javascript.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite code_injection_javascript as taint-flow pipeline"
```

---

### Task 4: JavaScript command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_javascript.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_javascript_finds_exec_call`, `command_injection_javascript_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_javascript_clean`:

```rust
#[test]
fn command_injection_javascript_no_fp_map_call() {
    let dir = tempfile::tempdir().unwrap();
    // .map() and .filter() — idiomatic array ops. Currently fires (all member calls match).
    std::fs::write(
        dir.path().join("test.js"),
        "const arr = [1, 2, 3];\nconst doubled = arr.map(x => x * 2);\nconst filtered = doubled.filter(x => x > 3);\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection"),
        "expected no command_injection finding for map/filter calls; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test**

Replace `command_injection_javascript_finds_exec_call` with:

```rust
#[test]
fn command_injection_javascript_finds_exec_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'req' is auto-tainted; cp.exec() is the sink.
    std::fs::write(
        dir.path().join("test.js"),
        "const cp = require('child_process');\nfunction run(req, res) {\n  const cmd = req.query.cmd;\n  cp.exec(cmd, (err, out) => { res.send(out); });\n}\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::JavaScript], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::JavaScript]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::JavaScript])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "exec_command_injection"),
        "expected command_injection/exec_command_injection finding for JavaScript; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_javascript -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_javascript_no_fp_map_call` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_javascript.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in JavaScript/TypeScript via taint analysis: user-controlled data reaching exec(), spawn(), or related child_process sinks",
  "languages": ["javascript", "jsx", "typescript", "tsx"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "req.body", "kind": "user_input"},
        {"pattern": "req.query", "kind": "user_input"},
        {"pattern": "req.params", "kind": "user_input"},
        {"pattern": "req.headers", "kind": "user_input"},
        {"pattern": "request.body", "kind": "user_input"},
        {"pattern": "request.query", "kind": "user_input"},
        {"pattern": "process.env", "kind": "env_var"},
        {"pattern": "process.argv", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": [
        {"pattern": "escape"},
        {"pattern": "sanitize"}
      ]
    },
    {
      "taint_sinks": [
        {"pattern": "exec", "vulnerability": "command_injection"},
        {"pattern": "execSync", "vulnerability": "command_injection"},
        {"pattern": "execFileSync", "vulnerability": "command_injection"},
        {"pattern": "spawn", "vulnerability": "command_injection"},
        {"pattern": "spawnSync", "vulnerability": "command_injection"},
        {"pattern": "execFile", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "exec_command_injection",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all JavaScript command_injection tests**

```
cargo test command_injection_javascript -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_javascript.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_javascript as taint-flow pipeline"
```

---

### Task 5: Go command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_go.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_go_finds_selector_call`, `command_injection_go_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_go_clean`:

```rust
#[test]
fn command_injection_go_no_fp_fmt_println() {
    let dir = tempfile::tempdir().unwrap();
    // fmt.Println — a selector call that is not exec.Command. Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nimport \"fmt\"\nfunc f(s string) { fmt.Println(s) }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".go")),
        "expected no command_injection finding for fmt.Println; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `args` parameter (auto-tainted)

Replace `command_injection_go_finds_selector_call` with:

```rust
#[test]
fn command_injection_go_finds_selector_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'args' is auto-tainted (PARAM_PATTERNS); exec.Command is the sink.
    std::fs::write(
        dir.path().join("test.go"),
        "package main\nimport \"os/exec\"\nfunc f(args string) { exec.Command(args) }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Go], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Go]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Go])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "exec_command_injection"),
        "expected command_injection/exec_command_injection finding for Go; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_go -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_go_no_fp_fmt_println` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_go.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in Go via taint analysis: user-controlled data reaching exec.Command or exec.CommandContext",
  "languages": ["go"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "r.URL.Query", "kind": "user_input"},
        {"pattern": "r.FormValue", "kind": "user_input"},
        {"pattern": "r.PostFormValue", "kind": "user_input"},
        {"pattern": "r.Header", "kind": "user_input"},
        {"pattern": "os.Getenv", "kind": "env_var"},
        {"pattern": "os.Args", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "exec.Command", "vulnerability": "command_injection"},
        {"pattern": "exec.CommandContext", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "exec_command_injection",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all Go command_injection tests**

```
cargo test command_injection_go -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_go.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_go as taint-flow pipeline"
```

---

### Task 6: Java command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_java.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_java_finds_method_invocation`, `command_injection_java_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_java_clean`:

```rust
#[test]
fn command_injection_java_no_fp_system_out() {
    let dir = tempfile::tempdir().unwrap();
    // System.out.println — a method call that is not exec(). Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.java"),
        "class A { void f(String s) { System.out.println(s); } }",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".java")),
        "expected no command_injection finding for System.out.println; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `args` parameter (auto-tainted); Java CFG extracts method name only, so `exec` matches `runtime.exec(args)`

Replace `command_injection_java_finds_method_invocation` with:

```rust
#[test]
fn command_injection_java_finds_method_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // 'args' is auto-tainted; Runtime.exec() call name = "exec" = sink.
    std::fs::write(
        dir.path().join("test.java"),
        "class A { void f(String args) throws Exception { Runtime.getRuntime().exec(args); } }",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Java], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Java]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Java])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for Java; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_java -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_java_no_fp_system_out` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_java.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in Java via taint analysis: user-controlled data reaching Runtime.exec() or ProcessBuilder",
  "languages": ["java"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "request.getParameter", "kind": "user_input"},
        {"pattern": "request.getParameterValues", "kind": "user_input"},
        {"pattern": "request.getHeader", "kind": "user_input"},
        {"pattern": "request.getInputStream", "kind": "user_input"},
        {"pattern": "request.getReader", "kind": "user_input"},
        {"pattern": "System.getenv", "kind": "env_var"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "exec", "vulnerability": "command_injection"},
        {"pattern": "new ProcessBuilder", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all Java command_injection tests**

```
cargo test command_injection_java -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_java.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_java as taint-flow pipeline"
```

---

### Task 7: C# command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_csharp.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_csharp_finds_invocation`, `command_injection_csharp_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_csharp_clean`:

```rust
#[test]
fn command_injection_csharp_no_fp_console_writeline() {
    let dir = tempfile::tempdir().unwrap();
    // Console.WriteLine — an invocation that is not Process.Start. Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.cs"),
        "class A { void F(string s) { Console.WriteLine(s); } }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".cs")),
        "expected no command_injection finding for Console.WriteLine; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `args` parameter

Replace `command_injection_csharp_finds_invocation` with:

```rust
#[test]
fn command_injection_csharp_finds_invocation() {
    let dir = tempfile::tempdir().unwrap();
    // 'args' is auto-tainted; Process.Start() is the sink.
    std::fs::write(
        dir.path().join("test.cs"),
        "using System.Diagnostics;\nclass A { void F(string args) { Process.Start(\"cmd.exe\", args); } }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::CSharp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::CSharp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::CSharp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for C#; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_csharp -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_csharp_no_fp_console_writeline` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_csharp.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in C# via taint analysis: user-controlled data reaching Process.Start or ProcessStartInfo",
  "languages": ["csharp"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "Request.Form", "kind": "user_input"},
        {"pattern": "Request.Query", "kind": "user_input"},
        {"pattern": "Request.Headers", "kind": "user_input"},
        {"pattern": "Request.Body", "kind": "user_input"},
        {"pattern": "Environment.GetEnvironmentVariable", "kind": "env_var"},
        {"pattern": "Console.ReadLine", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "Process.Start", "vulnerability": "command_injection"},
        {"pattern": "ProcessStartInfo", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all C# command_injection tests**

```
cargo test command_injection_csharp -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_csharp.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_csharp as taint-flow pipeline"
```

---

### Task 8: PHP command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/command_injection_php.json`
- Modify: `tests/audit_json_integration.rs` (functions: `command_injection_php_finds_function_call`, `command_injection_php_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `command_injection_php_clean`:

```rust
#[test]
fn command_injection_php_no_fp_strlen() {
    let dir = tempfile::tempdir().unwrap();
    // strlen() — a function call that is not a shell execution sink. Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($s) { return strlen($s); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "command_injection" && f.file_path.ends_with(".php")),
        "expected no command_injection finding for strlen; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `data` parameter (auto-tainted) which will be assigned from `$_GET`-like source

Replace `command_injection_php_finds_function_call` with:

```rust
#[test]
fn command_injection_php_finds_function_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'data' is auto-tainted (PARAM_PATTERNS); system() is the sink.
    std::fs::write(
        dir.path().join("test.php"),
        "<?php\nfunction f($data) { system($data); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Php], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Php]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Php])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "command_injection" && f.pattern == "command_injection_call"),
        "expected command_injection/command_injection_call finding for PHP; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test command_injection_php -- --nocapture 2>&1 | tail -20
```

Expected: `command_injection_php_no_fp_strlen` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/command_injection_php.json`**

```json
{
  "pipeline": "command_injection",
  "category": "security",
  "description": "Detect command injection in PHP via taint analysis: user-controlled data reaching shell execution functions",
  "languages": ["php"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "$_GET", "kind": "user_input"},
        {"pattern": "$_POST", "kind": "user_input"},
        {"pattern": "$_REQUEST", "kind": "user_input"},
        {"pattern": "$_SERVER", "kind": "user_input"},
        {"pattern": "$_COOKIE", "kind": "user_input"},
        {"pattern": "getenv", "kind": "env_var"}
      ]
    },
    {
      "taint_sanitizers": [
        {"pattern": "escapeshellarg"},
        {"pattern": "escapeshellcmd"},
        {"pattern": "filter_input"},
        {"pattern": "filter_var"}
      ]
    },
    {
      "taint_sinks": [
        {"pattern": "shell_exec", "vulnerability": "command_injection"},
        {"pattern": "exec", "vulnerability": "command_injection"},
        {"pattern": "system", "vulnerability": "command_injection"},
        {"pattern": "passthru", "vulnerability": "command_injection"},
        {"pattern": "popen", "vulnerability": "command_injection"},
        {"pattern": "proc_open", "vulnerability": "command_injection"},
        {"pattern": "pcntl_exec", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all PHP command_injection tests**

```
cargo test command_injection_php -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/command_injection_php.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite command_injection_php as taint-flow pipeline"
```

---

### Task 9: C command_injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/c_command_injection_c.json`
- Modify: `tests/audit_json_integration.rs` (functions: `c_command_injection_c_finds_call`, `c_command_injection_c_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `c_command_injection_c_clean`:

```rust
#[test]
fn c_command_injection_c_no_fp_strlen() {
    let dir = tempfile::tempdir().unwrap();
    // strlen() — not a shell execution sink. Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.c"),
        "#include <string.h>\nvoid f(char *s) { int n = strlen(s); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "c_command_injection" && f.file_path.ends_with(".c")),
        "expected no c_command_injection finding for strlen; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `args` parameter (auto-tainted)

Replace `c_command_injection_c_finds_call` with:

```rust
#[test]
fn c_command_injection_c_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'args' is auto-tainted (PARAM_PATTERNS); system() is the sink.
    std::fs::write(
        dir.path().join("test.c"),
        "#include <stdlib.h>\nvoid f(char *args) { system(args); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::C], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::C]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::C])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "c_command_injection" && f.pattern == "command_injection_call"),
        "expected c_command_injection/command_injection_call finding for C; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test c_command_injection -- --nocapture 2>&1 | tail -20
```

Expected: `c_command_injection_c_no_fp_strlen` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/c_command_injection_c.json`**

```json
{
  "pipeline": "c_command_injection",
  "category": "security",
  "description": "Detect command injection in C via taint analysis: user-controlled data reaching system(), popen(), or exec* functions",
  "languages": ["c"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "argv", "kind": "user_input"},
        {"pattern": "getenv", "kind": "env_var"},
        {"pattern": "fgets", "kind": "user_input"},
        {"pattern": "scanf", "kind": "user_input"},
        {"pattern": "fscanf", "kind": "user_input"},
        {"pattern": "read", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "system", "vulnerability": "command_injection"},
        {"pattern": "popen", "vulnerability": "command_injection"},
        {"pattern": "execv", "vulnerability": "command_injection"},
        {"pattern": "execvp", "vulnerability": "command_injection"},
        {"pattern": "execve", "vulnerability": "command_injection"},
        {"pattern": "execl", "vulnerability": "command_injection"},
        {"pattern": "execlp", "vulnerability": "command_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Command injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all C command_injection tests**

```
cargo test c_command_injection -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/c_command_injection_c.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite c_command_injection_c as taint-flow pipeline"
```

---

### Task 10: C++ injection taint rewrite

**Files:**
- Modify: `src/audit/builtin/cpp_injection_cpp.json`
- Modify: `tests/audit_json_integration.rs` (functions: `cpp_injection_cpp_finds_call`, `cpp_injection_cpp_clean`)

- [ ] **Step 1: Add anti-FP test**

Add after `cpp_injection_cpp_clean`:

```rust
#[test]
fn cpp_injection_cpp_no_fp_strlen() {
    let dir = tempfile::tempdir().unwrap();
    // strlen() — not a shell execution or format-string sink. Must NOT fire after fix.
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <string.h>\nvoid f(char *s) { int n = strlen(s); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        !findings.iter().any(|f| f.pipeline == "cpp_injection" && f.file_path.ends_with(".cpp")),
        "expected no cpp_injection finding for strlen; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern, &f.file_path)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Update positive test** — use `args` parameter

Replace `cpp_injection_cpp_finds_call` with:

```rust
#[test]
fn cpp_injection_cpp_finds_call() {
    let dir = tempfile::tempdir().unwrap();
    // 'args' is auto-tainted; system() is the sink.
    std::fs::write(
        dir.path().join("test.cpp"),
        "#include <cstdlib>\nvoid f(char *args) { system(args); }\n",
    ).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Cpp], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Cpp]).build().unwrap();
    let (findings, _) = AuditEngine::new()
        .languages(vec![Language::Cpp])
        .categories(vec!["security".to_string()])
        .run(&workspace, Some(&graph))
        .unwrap();
    assert!(
        findings.iter().any(|f| f.pipeline == "cpp_injection" && f.pattern == "command_injection_call"),
        "expected cpp_injection/command_injection_call finding for C++; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test cpp_injection -- --nocapture 2>&1 | tail -20
```

Expected: `cpp_injection_cpp_no_fp_strlen` FAILS.

- [ ] **Step 4: Rewrite `src/audit/builtin/cpp_injection_cpp.json`**

```json
{
  "pipeline": "cpp_injection",
  "category": "security",
  "description": "Detect command injection and format string injection in C++ via taint analysis",
  "languages": ["cpp"],
  "graph": [
    {
      "taint_sources": [
        {"pattern": "argv", "kind": "user_input"},
        {"pattern": "getenv", "kind": "env_var"},
        {"pattern": "cin", "kind": "user_input"},
        {"pattern": "std::cin", "kind": "user_input"},
        {"pattern": "fgets", "kind": "user_input"},
        {"pattern": "scanf", "kind": "user_input"},
        {"pattern": "getline", "kind": "user_input"},
        {"pattern": "std::getline", "kind": "user_input"}
      ]
    },
    {
      "taint_sanitizers": []
    },
    {
      "taint_sinks": [
        {"pattern": "system", "vulnerability": "command_injection"},
        {"pattern": "popen", "vulnerability": "command_injection"},
        {"pattern": "execv", "vulnerability": "command_injection"},
        {"pattern": "execvp", "vulnerability": "command_injection"},
        {"pattern": "execve", "vulnerability": "command_injection"},
        {"pattern": "printf", "vulnerability": "format_string_injection"},
        {"pattern": "fprintf", "vulnerability": "format_string_injection"},
        {"pattern": "sprintf", "vulnerability": "format_string_injection"},
        {"pattern": "snprintf", "vulnerability": "format_string_injection"},
        {"pattern": "vprintf", "vulnerability": "format_string_injection"},
        {"pattern": "vsprintf", "vulnerability": "format_string_injection"}
      ]
    },
    {
      "flag": {
        "pattern": "command_injection_call",
        "message": "Injection: tainted value from '{{tainted_var}}' reaches {{sink}} in {{file}}:{{line}}",
        "severity": "error"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all C++ injection tests**

```
cargo test cpp_injection -- --nocapture 2>&1 | tail -20
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/cpp_injection_cpp.json tests/audit_json_integration.rs
git commit -m "fix(audit): rewrite cpp_injection_cpp as taint-flow pipeline"
```

---

### Task 11: Rust panic_detection split

The pipeline `panic_detection_rust.json` fires on ALL method calls (false positive). Replace it with two focused pipelines. Existing tests that expect findings on `v.push(1)` and `s.trim().len()` are false-positive tests that must be inverted.

**Files:**
- Delete: `src/audit/builtin/panic_detection_rust.json`
- Create: `src/audit/builtin/panic_prone_calls_rust.json`
- Create: `src/audit/builtin/panic_prone_macros_rust.json`
- Modify: `tests/audit_json_integration.rs` — update all `panic_detection` tests

- [ ] **Step 1: Identify which tests test false-positive behavior**

These two tests assert a finding on code that should NOT fire after the fix:
- `panic_detection_rust_finds_method_call` — fixture: `fn f() { v.push(1); }` — `push` is not `unwrap`/`expect`
- `panic_detection_rust_chained_method` — fixture: `fn f(s: &str) -> usize { s.trim().len() }` — neither is a panic call

These must be converted from "expects finding" to "expects NO finding".

- [ ] **Step 2: Update tests** — all pipeline name references and invert the two FP tests

In `tests/audit_json_integration.rs`, make these changes:

**a) Rename `panic_detection_rust_finds_method_call` and invert its assertion:**

```rust
#[test]
fn panic_prone_calls_rust_ignores_push() {
    let dir = tempfile::tempdir().unwrap();
    // v.push(1) is not .unwrap()/.expect() — must NOT fire after fix.
    std::fs::write(dir.path().join("test.rs"), r#"fn f() { v.push(1); }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(!findings.iter().any(|f| f.pipeline == "panic_prone_calls_rust"),
        "expected no panic_prone_calls_rust finding for push; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}
```

**b) Rename `panic_detection_rust_chained_method` and invert its assertion:**

```rust
#[test]
fn panic_prone_calls_rust_ignores_trim_len() {
    let dir = tempfile::tempdir().unwrap();
    // s.trim().len() — neither is .unwrap()/.expect() — must NOT fire.
    std::fs::write(dir.path().join("test.rs"), r#"fn f(s: &str) -> usize { s.trim().len() }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(!findings.iter().any(|f| f.pipeline == "panic_prone_calls_rust"),
        "expected no panic_prone_calls_rust finding for trim().len(); got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}
```

**c) Update pipeline name in all remaining panic_detection tests** — replace every `f.pipeline == "panic_detection"` with `f.pipeline == "panic_prone_calls_rust"` and rename test functions. Example for `panic_detection_rust_finds_unwrap`:

```rust
#[test]
fn panic_prone_calls_rust_finds_unwrap() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.rs"), r#"fn f() { let x = Some(1).unwrap(); }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "panic_prone_calls_rust"),
        "expected panic_prone_calls_rust finding; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}
```

Apply the same rename (`panic_detection` → `panic_prone_calls_rust`) to all remaining 9 functions:
`finds_expect`, `no_findings_empty_fn`, `no_findings_struct_only`, `metadata_correct`, `multiple_calls`, `no_findings_constant`, `no_findings_use_only`, `findings_have_line`. Also add these new macro tests:

```rust
#[test]
fn panic_prone_macros_rust_finds_panic_macro() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.rs"), r#"fn f() { panic!("boom"); }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "panic_prone_macros_rust"),
        "expected panic_prone_macros_rust finding for panic!; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}

#[test]
fn panic_prone_macros_rust_finds_todo_macro() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.rs"), r#"fn f() { todo!() }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(findings.iter().any(|f| f.pipeline == "panic_prone_macros_rust"),
        "expected panic_prone_macros_rust finding for todo!; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}

#[test]
fn panic_prone_macros_rust_no_findings_empty_fn() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.rs"), r#"fn f() {}"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(!findings.iter().any(|f| f.pipeline == "panic_prone_macros_rust"),
        "expected no panic_prone_macros_rust finding for empty fn");
}
```

- [ ] **Step 3: Run tests — inverted FP tests must fail**

```
cargo test panic -- --nocapture 2>&1 | tail -30
```

Expected: `panic_prone_calls_rust_ignores_push` and `panic_prone_calls_rust_ignores_trim_len` FAIL (current broad pipeline fires on push/trim/len). All tests renamed to `panic_prone_calls_rust_*` also fail (pipeline name mismatch).

- [ ] **Step 4: Delete `panic_detection_rust.json` and create the two new files**

Delete:
```bash
rm src/audit/builtin/panic_detection_rust.json
```

Create `src/audit/builtin/panic_prone_calls_rust.json`:
```json
{
  "pipeline": "panic_prone_calls_rust",
  "category": "tech_debt",
  "description": "Detect .unwrap() and .expect() calls in Rust that may panic at runtime",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (field_expression field: (field_identifier) @method)) (#match? @method \"^(unwrap|expect)$\")"
    },
    {
      "flag": {
        "pattern": "panic_prone_call",
        "message": ".{{method}}() call may panic at runtime — consider using if let, match, or ? operator",
        "severity": "warning"
      }
    }
  ]
}
```

Create `src/audit/builtin/panic_prone_macros_rust.json`:
```json
{
  "pipeline": "panic_prone_macros_rust",
  "category": "tech_debt",
  "description": "Detect panic!(), todo!(), unimplemented!(), and unreachable!() macros in Rust production code",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(macro_invocation macro: (identifier) @name) (#match? @name \"^(panic|todo|unimplemented|unreachable)$\")"
    },
    {
      "flag": {
        "pattern": "panic_prone_macro",
        "message": "{{name}}!() macro will panic at runtime — remove before shipping to production",
        "severity": "warning"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all panic tests**

```
cargo test panic -- --nocapture 2>&1 | tail -30
```

Expected: all tests pass. `panic_prone_calls_rust_finds_unwrap`, `finds_expect`, etc. find via the new pipeline name. `ignores_push` and `ignores_trim_len` confirm no finding.

- [ ] **Step 6: Commit**

```bash
git add src/audit/builtin/panic_prone_calls_rust.json src/audit/builtin/panic_prone_macros_rust.json tests/audit_json_integration.rs
git rm src/audit/builtin/panic_detection_rust.json
git commit -m "fix(audit): split panic_detection_rust into panic_prone_calls and panic_prone_macros with #match? filters"
```

---

### Task 12: Rust clone_detection fix

**Files:**
- Modify: `src/audit/builtin/clone_detection_rust.json`
- Modify: `tests/audit_json_integration.rs` (function: `clone_detection_rust_method_call_detected`)

- [ ] **Step 1: Read the current `clone_detection_rust_method_call_detected` test**

```
cargo test clone_detection_rust_method_call_detected -- --nocapture 2>&1
```

Check what fixture it uses. If it tests a non-clone method (e.g., `.push()`, `.iter()`), it is a false-positive test and must be inverted. If it tests `.clone()`, it stays as-is and only needs no changes.

- [ ] **Step 2: Add anti-FP test**

Add after `clone_detection_rust_has_line_info`:

```rust
#[test]
fn clone_detection_rust_ignores_iter_push() {
    let dir = tempfile::tempdir().unwrap();
    // .iter() and .push() are not .clone()/.to_owned()/.to_string() — must NOT fire after fix.
    std::fs::write(dir.path().join("test.rs"),
        r#"fn f() { let mut v: Vec<i32> = Vec::new(); v.push(1); let _ = v.iter(); }"#).unwrap();
    let workspace = Workspace::load(dir.path(), &[Language::Rust], Some(10_000_000)).unwrap();
    let graph = GraphBuilder::new(&workspace, &[Language::Rust]).build().unwrap();
    let (findings, _) = AuditEngine::new().languages(vec![Language::Rust]).run(&workspace, Some(&graph)).unwrap();
    assert!(!findings.iter().any(|f| f.pipeline == "clone_detection"),
        "expected no clone_detection finding for iter/push; got: {:?}",
        findings.iter().map(|f| (&f.pipeline, &f.pattern)).collect::<Vec<_>>());
}
```

- [ ] **Step 3: Run tests — anti-FP test must fail**

```
cargo test clone_detection -- --nocapture 2>&1 | tail -20
```

Expected: `clone_detection_rust_ignores_iter_push` FAILS (current broad pattern fires on `.iter()` and `.push()`).

- [ ] **Step 4: Update `src/audit/builtin/clone_detection_rust.json`**

Replace the `match_pattern` value and fix the category. The new file:

```json
{
  "pipeline": "clone_detection",
  "category": "tech_debt",
  "description": "Detect overuse of .clone(), .to_owned(), and .to_string() that may indicate unnecessary allocations",
  "languages": ["rust"],
  "graph": [
    {
      "match_pattern": "(call_expression function: (field_expression field: (field_identifier) @method)) (#match? @method \"^(clone|to_owned|to_string)$\")"
    },
    {
      "flag": {
        "pattern": "unnecessary_clone",
        "message": ".{{method}}() call detected — consider borrowing or taking ownership instead",
        "severity": "info"
      }
    }
  ]
}
```

- [ ] **Step 5: Run all clone_detection tests**

```
cargo test clone_detection -- --nocapture 2>&1 | tail -20
```

Expected: all tests pass including `ignores_iter_push`.

- [ ] **Step 6: Run full test suite**

```
cargo test 2>&1 | tail -10
```

Expected: all tests pass with 0 failures.

- [ ] **Step 7: Commit**

```bash
git add src/audit/builtin/clone_detection_rust.json tests/audit_json_integration.rs
git commit -m "fix(audit): restrict clone_detection_rust to .clone()/.to_owned()/.to_string() with #match? filter"
```
