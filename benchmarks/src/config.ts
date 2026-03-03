import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import type { BenchmarkConfig, BenchmarkMode, FileConfig } from "./types";

const DEFAULT_CONFIG_PATH = "benchmark.config.json";
const DEFAULT_MODEL = "anthropic/claude-sonnet-4-20250514";
const DEFAULT_TIMEOUT = 300_000;
const DEFAULT_OUTPUT = "results";
const DEFAULT_VIRGIL_BIN = "virgil-cli";

const USAGE = `
virgil-benchmarks — Measure AI agent effectiveness with and without virgil-cli

USAGE
  bun run bench -- [options]

OPTIONS
  --repo <owner/name>      GitHub repo to benchmark against (required)
  --issue <number|random>  Issue number or "random" (default: random)
  --mode <mode>            baseline | virgil | both (default: both)
  --model <provider/model> Model for benchmark runs (default: ${DEFAULT_MODEL})
  --judge-model <p/model>  Model for AI-as-judge scoring (default: same as --model)
  --timeout <ms>           Timeout per session in ms (default: ${DEFAULT_TIMEOUT})
  --output <dir>           Output directory for results (default: ${DEFAULT_OUTPUT})
  --virgil-bin <path>      Path to virgil-cli binary (default: ${DEFAULT_VIRGIL_BIN})
  --config <path>          Path to config file (default: ${DEFAULT_CONFIG_PATH})
  --help                   Show this help message

ENVIRONMENT VARIABLES
  BENCH_REPO               Same as --repo
  BENCH_ISSUE              Same as --issue
  BENCH_MODE               Same as --mode
  BENCH_MODEL              Same as --model
  BENCH_JUDGE_MODEL        Same as --judge-model
  BENCH_TIMEOUT            Same as --timeout
  BENCH_OUTPUT             Same as --output
  BENCH_VIRGIL_BIN         Same as --virgil-bin

CONFIG FILE
  Create a benchmark.config.json in the benchmarks/ directory (see benchmark.config.schema.json).
  Priority: CLI args > env vars > config file > defaults

EXAMPLES
  bun run bench -- --repo facebook/react --issue 12345
  bun run bench -- --repo owner/repo --issue random --model anthropic/claude-sonnet-4-20250514
  bun run bench:baseline -- --repo owner/repo --issue 42
  BENCH_REPO=owner/repo bun run bench
`.trim();

function str(val: string | boolean | undefined): string | undefined {
  return typeof val === "string" ? val : undefined;
}

function loadFileConfig(configPath: string): FileConfig {
  const resolved = resolve(configPath);
  if (!existsSync(resolved)) return {};

  try {
    const raw = readFileSync(resolved, "utf-8");
    const parsed = JSON.parse(raw) as FileConfig;
    return parsed;
  } catch (err) {
    console.warn(`Warning: Could not parse config file ${resolved}: ${err}`);
    return {};
  }
}

export function parseConfig(): BenchmarkConfig {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      repo: { type: "string" },
      issue: { type: "string" },
      mode: { type: "string" },
      model: { type: "string" },
      "judge-model": { type: "string" },
      timeout: { type: "string" },
      output: { type: "string" },
      "virgil-bin": { type: "string" },
      config: { type: "string" },
      help: { type: "boolean", short: "h" },
    },
    strict: false,
    allowPositionals: true,
  });

  if (values.help) {
    console.log(USAGE);
    process.exit(0);
  }

  // Load config file (CLI --config > default path)
  const configPath = str(values.config) ?? DEFAULT_CONFIG_PATH;
  const file = loadFileConfig(configPath);

  // Resolve each field: CLI > env > file > default
  const repo =
    str(values.repo) ??
    process.env.BENCH_REPO ??
    (file.repo || undefined);

  if (!repo) {
    console.error(
      "Error: --repo is required (e.g. --repo facebook/react)\n" +
        "       Set it via CLI, BENCH_REPO env var, or benchmark.config.json",
    );
    process.exit(1);
  }

  const issueRaw =
    str(values.issue) ??
    process.env.BENCH_ISSUE ??
    (file.issue != null ? String(file.issue) : undefined) ??
    "random";
  const issue: number | "random" =
    issueRaw === "random" ? "random" : Number(issueRaw);
  if (issue !== "random" && (isNaN(issue) || issue <= 0)) {
    console.error("Error: --issue must be a positive number or 'random'");
    process.exit(1);
  }

  const modeRaw =
    str(values.mode) ??
    process.env.BENCH_MODE ??
    file.mode ??
    "both";
  const validModes: BenchmarkMode[] = ["baseline", "virgil", "both"];
  if (!validModes.includes(modeRaw as BenchmarkMode)) {
    console.error(`Error: --mode must be one of: ${validModes.join(", ")}`);
    process.exit(1);
  }

  const model =
    str(values.model) ??
    process.env.BENCH_MODEL ??
    file.model ??
    DEFAULT_MODEL;

  const judgeModel =
    str(values["judge-model"]) ??
    process.env.BENCH_JUDGE_MODEL ??
    (file.judgeModel || undefined) ??
    model; // falls back to benchmark model

  const timeout = Number(
    str(values.timeout) ??
      process.env.BENCH_TIMEOUT ??
      file.timeout ??
      DEFAULT_TIMEOUT,
  );

  const output =
    str(values.output) ??
    process.env.BENCH_OUTPUT ??
    file.output ??
    DEFAULT_OUTPUT;

  const virgilBin =
    str(values["virgil-bin"]) ??
    process.env.BENCH_VIRGIL_BIN ??
    file.virgilBin ??
    DEFAULT_VIRGIL_BIN;

  return {
    repo,
    issue,
    mode: modeRaw as BenchmarkMode,
    model,
    judgeModel,
    timeout,
    output,
    virgilBin,
  };
}

export function printConfig(config: BenchmarkConfig): void {
  console.log("\n=== Virgil-CLI Benchmark ===");
  console.log(`  Repo:        ${config.repo}`);
  console.log(`  Issue:       ${config.issue}`);
  console.log(`  Mode:        ${config.mode}`);
  console.log(`  Model:       ${config.model}`);
  console.log(`  Judge Model: ${config.judgeModel}`);
  console.log(`  Timeout:     ${config.timeout}ms`);
  console.log(`  Output:      ${config.output}`);
  console.log(`  Virgil Bin:  ${config.virgilBin}`);
  console.log();
}
