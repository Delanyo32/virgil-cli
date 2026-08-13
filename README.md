# virgil-cli

AI code review that reads your repository as a database instead of crawling files.

`virgil-cli scan` parses your code with [tree-sitter](https://tree-sitter.github.io/)
and writes the result into a local [DuckDB](https://duckdb.org/) database: every file,
every symbol, every call edge, every import, plus the full source text. It then starts
one AI agent per review topic. Each agent explores the codebase by writing SQL, not by
opening files one at a time, so it can ask "which functions have no callers?" or "which
files import each other in a cycle?" in a single query and get an answer over the whole
repository. Agents report findings; the CLI prints them grouped by review.

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

DuckDB is bundled into the binary, so you do not need to install a database.
No extension is ever downloaded at run time.

## Quickstart

```bash
export ANTHROPIC_API_KEY=sk-ant-...
virgil-cli scan .
```

The first run on a repository parses it, which takes a few seconds for a small project.
Later runs reuse the parsed database and start immediately.

A run prints findings grouped by review, worst first. Here is the layout, with made-up
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

## `scan` flags

| Flag | What it does | Default |
|---|---|---|
| `[PATH]` | Directory to scan | `.` (the current directory) |
| `--prompts <PATH>` | Use your own review prompt file, or a directory of them, instead of the four built-ins | built-ins |
| `--workers <N>` | How many review agents may run at the same time | `4` |
| `--provider <NAME>` | Which AI service to call: `anthropic`, `openai`, `ollama`, `openrouter` | `anthropic` |
| `--model <ID>` | Which model to use | `claude-opus-5` for `anthropic`; required for the rest |
| `--json` | Print findings as JSON instead of the grouped report | off |
| `--output <FILE>` | Also write a Markdown report to this file | off |
| `--rebuild` | Delete the cached database and parse the repository again | off |
| `--lang <LIST>` | Comma-separated extensions to parse, for example `ts,tsx,rs` | all supported |
| `-v`, `-vv`, `-vvv` | Show more log detail (info, debug, trace) | warnings only |
| `--quiet` | Show errors only | off |

`--json` and `--output` are independent. You can pass both: JSON goes to the terminal
and Markdown goes to the file.

There is no `--exclude` flag. File discovery already honours `.gitignore`, so ignored
files never reach the parser.

Two more commands round out the surface:

```bash
virgil-cli init-prompts <DIR>   # copy the built-in prompts into DIR so you can edit them
virgil-cli clean                # delete every cached database
```

Caches live in your operating system's cache directory, under a `virgil` folder:
`~/.cache/virgil` on Linux and `~/Library/Caches/virgil` on macOS. `init-prompts`
refuses to overwrite a prompt file that already exists, so it is safe to re-run.

## Custom reviews

A review is a Markdown file. Its filename becomes the review's name in the report, and
its text becomes the instruction the agent follows. The four built-ins are `security`,
`bugs`, `maintainability`, and `architecture`.

Copy them out, edit them, and point `--prompts` at the directory:

```bash
virgil-cli init-prompts ./reviews
$EDITOR ./reviews/security.md
virgil-cli scan . --prompts ./reviews
```

`--prompts` replaces the built-ins rather than adding to them. Pass a single `.md` file
to run exactly one review:

```bash
virgil-cli scan . --prompts ./reviews/security.md
```

Prompt files in a directory are read in sorted filename order, and only `.md` files
count.

## Providers

Pick the service with `--provider` and give it credentials through an environment
variable. An environment variable is a value your shell hands to the program, set with
`export NAME=value`.

| `--provider` | Environment variable | `--model` | Notes |
|---|---|---|---|
| `anthropic` (default) | `ANTHROPIC_API_KEY` | optional, defaults to `claude-opus-5` | Claude models |
| `openai` | `OPENAI_API_KEY` | required | OpenAI models |
| `openrouter` | `OPENROUTER_API_KEY` | required | Routes to many vendors through one key |
| `ollama` | none | required | Talks to `http://localhost:11434/v1`, so the model runs on your machine |

Ollama needs no key, but it does need the Ollama server running and the model already
pulled:

```bash
ollama pull qwen2.5:14b
virgil-cli scan . --provider ollama --model qwen2.5:14b
```

Pick an Ollama model that supports tool calling. Tool calling is the model's ability to
invoke the `query`, `read_source`, and `report_finding` functions. A model without it
cannot report anything.

## Security posture

The review agents never touch your filesystem or the network. They see one thing: a
local DuckDB database, opened with `enable_external_access=false`. That setting is a
DuckDB switch that blocks the database from reading files or making network requests,
so an agent cannot run `SELECT * FROM read_text('/etc/passwd')` or fetch a URL. The
switch is one-way; DuckDB refuses to turn it back on. On top of that, the `query` tool
accepts only statements starting with `SELECT` or `WITH`.

Your code does leave the machine in one way: the agent reads source snippets and puts
them in its prompts, and those prompts go to whichever model provider you chose. Use
`--provider ollama` to keep everything local.

## How it works

1. tree-sitter parses every supported file into a syntax tree, and extractors turn each
   tree into rows: symbols, call sites, imports, comments, types, scopes.
2. Those rows go into a DuckDB file in your cache directory, together with each file's
   complete source text, so nothing later needs to read the disk again.
3. One agent starts per review prompt. Each gets three tools: `query` runs read-only SQL
   against the database, `read_source` returns a line range of a file, and
   `report_finding` records one problem. Agents run in parallel, capped by `--workers`.

The parsed database is cached by the absolute path of the directory you scanned. Scan
the same directory again and the run skips parsing entirely. Pass `--rebuild` after
changing code, because there is no incremental update: the cache is either reused whole
or rebuilt whole.

Coverage is not equal across languages. Import resolution currently produces nothing on
Rust repositories, and inheritance edges are sparse, so the `architecture` review has
less to work with there than on a TypeScript repository. Agents are told to count a
table's rows before building a finding on it, so a thin table means fewer findings, not
invented ones.

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

## License

MIT
