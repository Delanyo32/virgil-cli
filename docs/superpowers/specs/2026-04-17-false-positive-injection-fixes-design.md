# False Positive Injection Fixes Design

**Date:** 2026-04-17
**Status:** Approved

## Problem

Cross-language benchmarks show "Extra" (false positive) counts ranging from 1,263 (Go) to 17,801 (Python) per run. The dominant sources are:

1. **Injection pipelines (10 files, 8 languages)** — every injection pipeline was written without `#match?` predicate support and falls back to matching all function calls / all method invocations / all member expressions. A Python codebase fires `code_injection_call` on every `len()`, `print()`, `range()` call. A JavaScript codebase fires `exec_command_injection` on every `.map()`, `.filter()`, `.push()` call. The fix is to rewrite each pipeline as a proper taint-flow analysis following the SQL injection reference pattern already used in this codebase.

2. **Rust pipelines (2 files)** — `panic_detection_rust.json` and `clone_detection_rust.json` both match every `field_expression` method call (all of `.iter()`, `.collect()`, `.push()`, etc.) instead of restricting to the target methods. Both also use `"category": "code-quality"` which is not a valid category string. The fix is `#match?` name restriction + category correction.

Scope: injection pipelines and the two Rust pipelines only. SQL injection pipelines are not changed (they already use taint flow correctly and appear in Coverage Wins).

---

## Approach

All injection pipelines are rewritten to `taint_sources` → `taint_sanitizers` → `taint_sinks` → `flag`, following the structure of `sql_injection_python.json` and `sql_injection_javascript.json` exactly. A finding is only emitted when a variable derived from an external source reaches a dangerous sink without passing through a sanitizer.

Rust pipelines are fixed with `#match?` predicates and split where needed for clean separation.

---

## Section 1: Injection Pipeline Taint-Flow Rewrites

### 1a: Python — `code_injection_python.json`

**Sinks:** `eval(`, `exec(`, `compile(`

**Sources:**
```
request.form, request.args, request.data, request.json, request.values (Flask)
request.POST, request.GET, request.body (Django)
input(, sys.argv, os.environ, os.environ.get
```

**Sanitizers:** none defined (no widely-used safe-eval equivalent in Python; users should not call eval/exec on user input at all)

**Finding pattern:** `code_injection` / severity `error`

---

### 1b: Python — `command_injection_python.json`

**Sinks:** `os.system(`, `os.popen(`, `subprocess.run(`, `subprocess.call(`, `subprocess.Popen(`, `subprocess.check_output(`, `subprocess.check_call(`

**Sources:** same as 1a above

**Sanitizers:** `shlex.quote(`

**Finding pattern:** `command_injection` / severity `error`

---

### 1c: JavaScript/TypeScript — `code_injection_javascript.json`

Pipeline already has `"languages": ["javascript", "jsx", "typescript", "tsx"]` — no separate TypeScript file needed.

**Sinks:** `eval(`, `Function(`, `vm.runInContext(`, `vm.runInNewContext(`, `vm.runInThisContext(`

**Sources:**
```
req.body, req.query, req.params, req.headers (Express)
request.body, request.query (generic)
process.env, process.argv
```

**Sanitizers:** `escape(`, `encodeURIComponent(`

**Finding pattern:** `code_injection` / severity `error`

---

### 1d: JavaScript/TypeScript — `command_injection_javascript.json`

Pipeline already covers `typescript`/`tsx` — no separate TypeScript file needed.

**Sinks:** `exec(`, `execSync(`, `execFileSync(`, `spawn(`, `spawnSync(`, `execFile(`

**Sources:** same as 1c above

**Sanitizers:** `escape(`, `sanitize(`

**Finding pattern:** `exec_command_injection` / severity `error`

---

### 1e: Go — `command_injection_go.json`

**Sinks:** `exec.Command(`, `exec.CommandContext(`

**Sources:**
```
r.URL.Query(, r.FormValue(, r.PostFormValue(, r.Header (net/http request)
os.Getenv(, os.Args
```

**Sanitizers:** none defined (Go has no standard shell-escape library; `filepath.Clean` is for path traversal, not command injection)

**Finding pattern:** `exec_command_injection` / severity `error`

---

### 1f: Java — `command_injection_java.json`

**Sinks:** `Runtime.exec(`, `new ProcessBuilder(`

**Sources:**
```
request.getParameter(, request.getParameterValues(, request.getHeader(
request.getInputStream(, request.getReader(
System.getenv(, args
```

**Sanitizers:** none defined for command execution context

**Finding pattern:** `command_injection` / severity `error`

---

### 1g: C# — `command_injection_csharp.json`

**Sinks:** `Process.Start(`, `new ProcessStartInfo(`

**Sources:**
```
Request.Form, Request.Query, Request.Headers, Request.Body (ASP.NET)
Environment.GetEnvironmentVariable(
Console.ReadLine(, args
```

**Sanitizers:** none defined for command execution context

**Finding pattern:** `command_injection` / severity `error`

---

### 1h: PHP — `command_injection_php.json`

**Sinks:** `shell_exec(`, `exec(`, `system(`, `passthru(`, `popen(`, `proc_open(`, `pcntl_exec(`

**Sources:** `$_GET`, `$_POST`, `$_REQUEST`, `$_SERVER`, `$_COOKIE`, `getenv(`

**Sanitizers:** `escapeshellarg(`, `escapeshellcmd(`, `filter_input(`, `filter_var(`

**Finding pattern:** `command_injection` / severity `error`

---

### 1i: C — `c_command_injection_c.json`

**Sinks:** `system(`, `popen(`, `execv(`, `execvp(`, `execve(`, `execl(`, `execlp(`

**Sources:** `argv`, `getenv(`, `fgets(`, `scanf(`, `fscanf(`, `read(`

**Sanitizers:** none defined (C has no standard shell-escape function)

**Finding pattern:** `command_injection` / severity `error`

---

### 1j: C++ — `cpp_injection_cpp.json`

Covers two injection types: command injection via shell execution functions and format string injection via printf family with user-controlled format strings.

**Command sinks:** `system(`, `popen(`, `execv(`, `execvp(`, `execve(`

**Format string sinks:** `printf(`, `fprintf(`, `sprintf(`, `snprintf(`, `vprintf(`, `vsprintf(`

**Sources:** `argv`, `getenv(`, `cin`, `std::cin`, `fgets(`, `scanf(`, `getline(`, `std::getline(`

**Sanitizers:** none defined

**Finding pattern:** `command_injection` / severity `error`

---

## Section 2: Rust Pipeline Fixes

### 2a: `panic_detection_rust.json` → split into two pipelines

**Reason for split:** A single pipeline cannot cleanly express both a method-call pattern and a macro-invocation pattern in one `match_pattern` stage and emit a unified finding. Two focused pipelines are cleaner.

**`panic_prone_calls_rust.json`** (new name, replaces `panic_detection_rust.json`):
```json
{
  "pipeline": "panic_prone_calls_rust",
  "category": "tech_debt",
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

**`panic_prone_macros_rust.json`** (new file):
```json
{
  "pipeline": "panic_prone_macros_rust",
  "category": "tech_debt",
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

The old `panic_detection_rust.json` is deleted.

---

### 2b: `clone_detection_rust.json`

**Change:** Add `#match?` predicate to restrict to `.clone()`, `.to_owned()`, `.to_string()`.
**Change:** Fix `"category": "code-quality"` → `"tech_debt"`.

Updated `match_pattern`:
```
(call_expression function: (field_expression field: (field_identifier) @method)) (#match? @method "^(clone|to_owned|to_string)$")
```

---

## Changes Summary

| Type | Count | Files |
|---|---|---|
| JSON (injection rewrites) | 10 files | code_injection_python, command_injection_python, code_injection_javascript, command_injection_javascript, command_injection_go, command_injection_java, command_injection_csharp, command_injection_php, c_command_injection_c, cpp_injection_cpp |
| JSON (Rust split: delete + 2 new) | 3 files | panic_detection_rust.json deleted; panic_prone_calls_rust.json + panic_prone_macros_rust.json added |
| JSON (Rust edit) | 1 file | clone_detection_rust.json |

**Total: 14 file changes (10 rewrites, 1 delete, 2 new, 1 edit). No Rust code changes.**

---

## Out of Scope

- SQL injection pipelines (already use taint flow; no changes)
- `reflection_injection_java.json` (separate reflection attack surface; needs separate review)
- Cross-file taint tracking (taint context is file-scoped; cross-file flows not detected by this approach, consistent with SQL injection behavior)
- `print_instead_of_logging`, `hardcoded_secrets`, `deprecated_api_usage` detection (separate detection gap work)
- `lhs_is_parameter` DSL primitive for `argument_mutation` (already deferred in prior spec)
