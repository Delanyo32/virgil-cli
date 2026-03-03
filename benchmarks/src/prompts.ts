import type { GitHubIssue } from "./types";

export function buildBaselinePrompt(issue: GitHubIssue, repoPath: string): string {
  return `You are an expert software engineer. You have access to a codebase cloned at: ${repoPath}

Your task is to analyze the following GitHub issue and produce a detailed implementation plan.

## GitHub Issue #${issue.number}: ${issue.title}

${issue.body}

## Instructions

1. Explore the codebase using the tools available to you (read files, search for patterns, list files, run shell commands).
2. Understand the project structure, relevant code, and how the issue relates to the existing codebase.
3. Produce a detailed implementation plan that includes:
   - Which files need to be modified or created
   - What specific changes need to be made in each file (function names, line numbers where possible)
   - The order of changes
   - Any potential risks or edge cases
   - Testing strategy

Be as specific as possible. Reference actual file paths, function names, and code structures you discover.

## Output

Provide your implementation plan as a structured document with clear sections.`;
}

export function buildVirgilPrompt(
  issue: GitHubIssue,
  repoPath: string,
  virgilBin: string,
  dataDir?: string,
): string {
  const dd = dataDir ?? "output";
  return `You are an expert software engineer. You have access to a codebase cloned at: ${repoPath}

You also have access to \`${virgilBin}\`, a codebase intelligence tool. The codebase has already been parsed — do NOT run \`${virgilBin} parse\`. The parsed data is at \`${dd}\`.

## Exploration Methodology

Follow these phases in order. Each phase builds on the previous one to maximize efficiency and minimize unnecessary file reading.

### Phase 1: Orientation

Start by running the overview command to understand the codebase at a high level:
\`\`\`bash
${virgilBin} overview --data-dir ${dd}
\`\`\`
This gives you: language breakdown, directory structure, top symbols, and a dependency summary. Use this to orient yourself before diving deeper.

### Phase 2: Targeted Exploration

Use these commands to find relevant code WITHOUT reading files:
- \`${virgilBin} search <query> --data-dir ${dd}\` — Fuzzy search for symbols by name
- \`${virgilBin} outline <file_path> --data-dir ${dd}\` — Show all symbols in a file with line numbers
- \`${virgilBin} files --data-dir ${dd}\` — List all parsed files
- \`${virgilBin} deps <file_path> --data-dir ${dd}\` — Show what a file imports
- \`${virgilBin} dependents <file_path> --data-dir ${dd}\` — Show what files import a given file
- \`${virgilBin} callers <symbol_name> --data-dir ${dd}\` — Find which files import a specific symbol
- \`${virgilBin} imports --data-dir ${dd}\` — List all imports (filterable with --module, --kind, --file, --external, --internal)

### Phase 3: Precise Reading

Once you know exactly what to look at, read only the relevant lines:
\`\`\`bash
${virgilBin} read <file_path> --root ${repoPath} --start-line <N> --end-line <M> --data-dir ${dd}
\`\`\`
**Do NOT read entire files.** Always run \`outline\` first to find the line numbers of the symbol you care about, then use \`--start-line\`/\`--end-line\` to read just that section.

### Phase 4: Deep Analysis (if needed)

For complex queries, use raw SQL against the parsed data:
\`\`\`bash
${virgilBin} query "<SQL>" --data-dir ${dd}
\`\`\`
Available tables: \`files\`, \`symbols\`, \`imports\`, \`comments\`. Example queries:
- \`SELECT file_path, name, kind FROM symbols WHERE name LIKE '%Handler%'\`
- \`SELECT DISTINCT source FROM imports WHERE file_path = 'src/index.ts'\`
- \`SELECT s.file_path, s.name FROM symbols s JOIN imports i ON s.file_path = i.file_path WHERE i.source LIKE '%express%'\`

## Guidelines

- **Prefer virgil-cli over grep/glob** for finding symbols, tracing dependencies, and understanding structure.
- **Always outline before read** — get line numbers first, then read only the lines you need.
- **Use deps/dependents for tracing** — understand how files connect before reading them.
- **Standard tools are still available** — use file reading, grep, and shell commands when virgil-cli is not the right fit (e.g., reading config files, checking git history).

## GitHub Issue #${issue.number}: ${issue.title}

${issue.body}

## Instructions

1. Follow the phased exploration methodology above to understand the codebase.
2. Identify the relevant code, files, and dependencies related to this issue.
3. Produce a detailed implementation plan that includes:
   - Which files need to be modified or created
   - What specific changes need to be made in each file (function names, line numbers where possible)
   - The order of changes
   - Any potential risks or edge cases
   - Testing strategy

Be as specific as possible. Reference actual file paths, function names, and code structures you discover.

## Output

Provide your implementation plan as a structured document with clear sections.`;
}

export function buildScoringPrompt(
  issue: GitHubIssue,
  planText: string,
  mode: "baseline" | "virgil",
): string {
  return `You are an expert code reviewer acting as a judge. Score the following implementation plan on 5 dimensions.

## Context

A software engineer was given this GitHub issue and asked to produce an implementation plan by exploring the codebase.

### GitHub Issue #${issue.number}: ${issue.title}

${issue.body}

### Mode: ${mode}
${mode === "virgil" ? "The engineer had access to virgil-cli (a codebase intelligence tool) in addition to standard tools." : "The engineer used only standard code exploration tools (file reading, grep, glob, bash)."}

### Implementation Plan

${planText}

## Scoring Rubric

Score each dimension from 0 to 100:

1. **completeness** (weight: 25%) — Does the plan address all aspects of the issue? Are there missing steps or overlooked requirements?
2. **accuracy** (weight: 25%) — Are file paths, symbol names, and technical claims correct based on your understanding? Does it reference real code constructs?
3. **specificity** (weight: 20%) — Does the plan provide file/line/function-level guidance, or is it vague and hand-wavy?
4. **feasibility** (weight: 15%) — Is the plan practically implementable? Are the steps in a logical order? Are there obvious flaws?
5. **fileIdentification** (weight: 15%) — Did the plan correctly identify the right files that need to be modified?

## Output

Respond with ONLY a JSON object matching this exact schema (no markdown, no extra text):

{
  "completeness": <0-100>,
  "accuracy": <0-100>,
  "specificity": <0-100>,
  "feasibility": <0-100>,
  "fileIdentification": <0-100>,
  "rationale": "<brief explanation of scoring>"
}`;
}

export const SCORE_JSON_SCHEMA = {
  type: "object" as const,
  properties: {
    completeness: { type: "number" as const },
    accuracy: { type: "number" as const },
    specificity: { type: "number" as const },
    feasibility: { type: "number" as const },
    fileIdentification: { type: "number" as const },
    rationale: { type: "string" as const },
  },
  required: [
    "completeness",
    "accuracy",
    "specificity",
    "feasibility",
    "fileIdentification",
    "rationale",
  ],
};
