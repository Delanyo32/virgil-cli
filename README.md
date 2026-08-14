# virgil-cli

Code review that checks the rules you write in plain English.

virgil-cli turns your whole repository into a local database, then points AI agents at
it. You write each review as a Markdown file in plain English. The agents query the
database, read the source to confirm, and report what is wrong, with the file and line.

```bash
export OPENROUTER_API_KEY=sk-or-...
virgil-cli scan .
```

## Why this instead of a linter

A linter checks one file at a time against rules someone else shipped. Two things follow
from that, and virgil-cli is built to fix both.

**It cannot see across your whole repository.** virgil-cli parses everything into one
database before any review starts. Every file, function, call, and import is a row. So
a check like "which exported functions does nothing import?" is one query over the whole
repository, not two thousand file reads.

**It only knows the rules it shipped with.** Here, the rules are yours. Write your team's
convention as a sentence in a `.md` file, and it gets checked on every scan.

What you get back is a list of problems, sorted worst first. It judges your code. It does
not explain it.

## What people use it for

**Before a big refactor.** Find the dead exports, the import cycles, and the files
everything depends on, so you know what will hurt to move.

**Onboarding to an unfamiliar repo.** The architecture review names the god files, the
import cycles, and the dead exports. That tells you which files are load-bearing before
you touch them. It is a list of problems, not a tour of the codebase.

**Enforcing team rules that no linter knows about.** "Every database write goes through
the repository layer." "No new code calls the legacy client." Write the sentence, save
it as a `.md` file, and it gets checked from then on.

**A second pass on a security review.** The built-in security review traces risky
functions back through the call graph to find who reaches them, and reads the real
source before it reports anything.

**Codebases you inherited.** Point it at the mess and get a ranked list of what is
actually wrong, not five thousand style warnings.

## Install

### macOS / Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Delanyo32/virgil-cli/master/install.sh | sh
```

The script downloads a prebuilt binary into `~/.local/bin`. Pass `-b DIR` to install
somewhere else. Released targets are `x86_64` and `aarch64` for both Linux (glibc) and
macOS.

> Windows is not supported yet. The bundled DuckDB C++ build hits an MSVC removal of
> `stdext::checked_array_iterator` in VS 2026. `stdext::checked_array_iterator` is a
> Microsoft compiler helper that DuckDB's vendored code still uses. The fix belongs
> upstream, so use WSL (Windows Subsystem for Linux) for now.

### From source

```bash
cargo install --path .
```

The database is built into the binary. You do not need to install or run one yourself.

## Your first scan

```bash
export OPENROUTER_API_KEY=sk-or-...
virgil-cli scan .
```

The first run parses the repository, which takes a few seconds on a small project.
Later runs reuse the parsed database and start right away.

You get findings grouped by review, worst first. Here is the layout, with made-up
findings standing in for real ones:

```
security (2 findings)
  HIGH  src/api/users.ts:118                     the WHERE clause is built by string concatenation
  LOW   src/config/load.ts:24                    the fallback secret is hard-coded

bugs (1 finding)
  MED   src/queue/worker.ts:57                   the retry loop swallows the error and returns success
```

`HIGH`, `MED`, `LOW`, and `INFO` are severity labels the agent picks. The number after
each review name counts that review's findings. Each line is
`severity  file:line  what is wrong`. When nothing is found, the run prints
`No findings.`

Four reviews run by default:

| Review | What it looks for |
|---|---|
| `security` | Injection, hard-coded secrets, missing input validation, `eval`-style execution, weak hashing |
| `bugs` | Null misuse, swallowed errors, off-by-one mistakes, callers passing what the callee does not expect |
| `maintainability` | Oversized functions, deep nesting, duplication, misleading names, dead exports |
| `architecture` | Import cycles, layering violations, god files, dead exports, inheritance tangles |

## Write your own review

This is the part most people came for. A review is one Markdown file. The filename
becomes the review's name in the report. The text inside becomes the instruction the
agent follows. That is the whole format.

Start from the built-ins:

```bash
virgil-cli init-prompts ./reviews
$EDITOR ./reviews/security.md
virgil-cli scan . --prompts ./reviews
```

Or write one from scratch. Save this as `./reviews/our-rules.md`:

```markdown
Check that every function writing to the database goes through `src/repo/`.
Anything calling the raw client from outside that folder is a finding.
Read the real source before reporting. Severity: high if it writes, medium if it reads.
```

Then:

```bash
virgil-cli scan . --prompts ./reviews/our-rules.md
```

Three things worth knowing:

1. `--prompts` replaces the built-in reviews. It does not add to them. To keep the
   built-ins too, run `init-prompts` into your folder first.
2. Point it at a single `.md` file to run exactly one review. Point it at a folder to
   run every `.md` file in it, in sorted filename order.
3. There is no limit on how many reviews you can write. `--workers` controls how many
   run at the same time, and the rest wait their turn. Each review is a full AI agent
   run, so cost grows with the count.

Prompts that work well tell the agent to read the real source before reporting, and say
what each severity level means to you. The built-ins do both. Copy that shape.

## `scan` flags

| Flag | What it does | Default |
|---|---|---|
| `[PATH]` | Directory to scan | `.` (the current directory) |
| `--prompts <PATH>` | Use your own review file, or a folder of them, instead of the four built-ins | built-ins |
| `--workers <N>` | How many review agents may run at the same time | `4` |
| `--provider <NAME>` | Which AI service to call: `openrouter`, `anthropic`, `openai`, `ollama` | `openrouter` |
| `--model <ID>` | Which model to use | `z-ai/glm-4.6` for `openrouter`, `claude-opus-5` for `anthropic`; required for the rest |
| `--json` | Print findings as JSON instead of the grouped report | off |
| `--output <FILE>` | Also write a Markdown report to this file | off |
| `--rebuild` | Delete the cached database and parse the repository again | off |
| `-l`, `--lang <LIST>` | Comma-separated extensions to parse, for example `ts,tsx,rs` | all supported |
| `-v`, `-vv`, `-vvv` | Show more log detail (info, debug, trace) | warnings only |
| `--quiet` | Show errors only | off |

`--json` and `--output` are independent. You can pass both. JSON goes to the terminal
and Markdown goes to the file.

There is no `--exclude` flag. File discovery already honours `.gitignore`, so ignored
files never reach the parser.

Two more commands round out the surface:

```bash
virgil-cli init-prompts <DIR>   # copy the built-in reviews into DIR so you can edit them
virgil-cli clean                # delete every cached database
```

Caches live in your operating system's cache directory, under a `virgil` folder:
`~/.cache/virgil` on Linux and `~/Library/Caches/virgil` on macOS. `init-prompts`
refuses to overwrite a file that already exists, so it is safe to re-run.

## Picking a provider

Pick the service with `--provider` and give it credentials through an environment
variable. An environment variable is a value your shell hands to the program, set with
`export NAME=value`.

| `--provider` | Environment variable | `--model` | Notes |
|---|---|---|---|
| `openrouter` (default) | `OPENROUTER_API_KEY` | optional, defaults to `z-ai/glm-4.6` | Routes to many vendors through one key |
| `anthropic` | `ANTHROPIC_API_KEY` | optional, defaults to `claude-opus-5` | Claude models |
| `openai` | `OPENAI_API_KEY` | required | OpenAI models |
| `ollama` | none | required | Talks to `http://localhost:11434/v1`, so the model runs on your machine |

OpenRouter is the default because one key reaches every vendor, and the default model is
cheap enough to run the full four-review scan without thinking about it. Get a key at
[openrouter.ai/keys](https://openrouter.ai/keys).

### Swapping the model

Any OpenRouter model works, as long as it supports tool calling. Tool calling is the
model's ability to invoke the `query`, `read_source`, and `report_finding` functions. A
model without it cannot report anything.

Some verified alternatives, with the price OpenRouter lists per million tokens. "In" is
what you pay for text sent to the model. "Out" is what you pay for text it writes back.

| `--model` | In | Out | Context |
|---|---|---|---|
| `z-ai/glm-4.6` (default) | $0.50 | $2.00 | 204k |
| `z-ai/glm-4.7` | $0.40 | $1.75 | 204k |
| `qwen/qwen3-coder` | $0.30 | $1.00 | 262k |
| `qwen/qwen3-coder-plus` | $0.65 | $3.25 | 1M |
| `z-ai/glm-5.2` | $0.63 | $1.98 | 1M |

Context is how much text the model can hold at once. A bigger window helps on large
files, but the agents query the database instead of reading everything, so 204k is
enough for most repositories.

```bash
virgil-cli scan . --model qwen/qwen3-coder
```

Ollama needs no key, but it does need the Ollama server running and the model already
pulled:

```bash
ollama pull qwen2.5:14b
virgil-cli scan . --provider ollama --model qwen2.5:14b
```

The same tool-calling rule applies here. Pick an Ollama model that supports it.

Each review gives up after 10 minutes and is reported as a failed review. The other
reviews keep going, and anything that review already found is still printed.

## Does my code leave my machine?

Only in one way, and you can turn it off.

The review agents cannot touch your filesystem or the network. They see one thing: a
local database opened with `enable_external_access=false`. That setting is a DuckDB
switch that blocks the database from reading files or making network requests, so an
agent cannot run `SELECT * FROM read_text('/etc/passwd')` or fetch a URL. The switch is
one-way. DuckDB refuses to turn it back on. On top of that, the query tool accepts only
statements starting with `SELECT` or `WITH`.

What does leave is this: the agent reads source snippets and puts them in its prompts,
and those prompts go to whichever model provider you chose. Use `--provider ollama` to
keep everything on your machine.

## How it works

1. [tree-sitter](https://tree-sitter.github.io/) parses every supported file into a
   syntax tree. Extractors turn each tree into rows: symbols, call sites, imports,
   comments, types, scopes.
2. Those rows go into a [DuckDB](https://duckdb.org/) file in your cache directory,
   together with each file's complete source text. Nothing later needs to read the disk
   again.
3. One agent starts per review. Each gets three tools. `query` runs read-only SQL
   against the database. `read_source` returns a line range of a file. `report_finding`
   records one problem. Agents run in parallel, capped by `--workers`.

The parsed database is cached by the absolute path of the folder you scanned. Scan the
same folder again and the run skips parsing. Pass `--rebuild` after changing code,
because there is no incremental update. The cache is either reused whole or rebuilt
whole.

## Supported languages

| Language | Extensions |
|----------|------------|
| TypeScript | `.ts` |
| TSX | `.tsx` |
| JavaScript | `.js` |
| JSX | `.jsx` |
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` |
| C# | `.cs` |
| Rust | `.rs` |
| Python | `.py`, `.pyi` |
| Go | `.go` |
| Java | `.java` |
| PHP | `.php` |

`.h` maps to C on purpose. Name C++ headers `.hpp`, `.hxx`, or `.hh` so they parse as
C++.

Files in any other language get no row in the database at all. Agents are told this, and
told not to claim anything about `.env` files, YAML, Dockerfiles, or lockfiles, because
they cannot see them.

## Known limits

**You get findings, not a report.** Every result is one problem: a severity, a file, a
line, and a sentence. There is no summary, no diagram, and no way to ask the tool a
question and read an answer back. If you want something described rather than judged,
write a review prompt that asks for it, and you still get it back as findings.

**Line numbers are not guaranteed.** A finding carries an optional line. When the agent
does not supply one, the report prints the file path alone.

**Scan the repository root, not a subfolder.** Import resolution depends on the paths it
sees. Scanning this repository at `.` resolved 138 Rust imports. Scanning the same code
at `./src` resolved 0, because the paths no longer started with `src/`. Use `--lang` to
narrow a scan instead of pointing at a subfolder.

**Import coverage varies a lot by language.** Measured on real cached scans, counting
resolved import rows against the raw imports the parser saw:

| Repository | Raw | Resolved |
|---|---|---|
| Go / C / C++, 1093 files | 6,626 | 30,717 |
| Rust, 78 files, scanned at `.` | 719 | 138 |
| Rust, 70 files, scanned at `./src` | 698 | 0 |

Go resolves to more rows than it saw because one package import fans out to every file
in that package. Rust resolves about one in five. So the `architecture` review, which
runs on the import graph, is strongest on Go and C-family code and thinnest on Rust.

**Inheritance links are sparse.** The `extends` table had 49 rows on the Go/C++ repo and
0 on both Rust repos. Reviews leaning on inheritance find little.

**Agents are told to count a table's rows before building a finding on it.** A thin table
means fewer findings, not invented ones.

**There is no incremental reparse.** Use `--rebuild` after your code changes.

## License

MIT
