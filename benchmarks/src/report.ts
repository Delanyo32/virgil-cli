import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { BenchmarkReport, BenchmarkRun, MetricDelta, LogEntry } from "./types";

export function printRunSummary(run: BenchmarkRun): void {
  const { score, metrics } = run;
  console.log(`\n  Mode: ${run.mode.toUpperCase()}`);
  console.log(`  Weighted Score: ${score.weighted}`);
  console.log(`    Completeness:      ${score.dimensions.completeness}`);
  console.log(`    Accuracy:          ${score.dimensions.accuracy}`);
  console.log(`    Specificity:       ${score.dimensions.specificity}`);
  console.log(`    Feasibility:       ${score.dimensions.feasibility}`);
  console.log(`    File ID:           ${score.dimensions.fileIdentification}`);
  console.log(`  Rationale: ${score.rationale}`);
  console.log(`  Tokens: ${metrics.totalTokens} (prompt: ${metrics.promptTokens}, completion: ${metrics.completionTokens})`);
  console.log(`  Cost: $${metrics.costUsd.toFixed(4)}`);
  console.log(`  Time: ${(metrics.wallClockMs / 1000).toFixed(1)}s`);
  console.log(`  Files Read: ${metrics.filesRead}`);
  console.log(`  Glob Calls: ${metrics.globCalls}`);
  console.log(`  Bash Calls: ${metrics.bashCalls}`);
  console.log(`  Tool Calls: ${metrics.toolCalls} (${Object.entries(metrics.toolBreakdown).map(([k, v]) => `${k}: ${v}`).join(", ")})`);
  console.log(`  Message Rounds: ${metrics.messageRounds}`);
}

export function printComparisonTable(deltas: MetricDelta[]): void {
  console.log("\n" + "=".repeat(78));
  console.log("  COMPARISON: Baseline vs Virgil-Enhanced");
  console.log("=".repeat(78));

  const labelWidth = 20;
  const numWidth = 12;

  const header = [
    "Metric".padEnd(labelWidth),
    "Baseline".padStart(numWidth),
    "Virgil".padStart(numWidth),
    "Delta".padStart(numWidth),
    "Delta %".padStart(numWidth),
  ].join(" | ");

  console.log(header);
  console.log("-".repeat(header.length));

  for (const d of deltas) {
    const sign = d.delta >= 0 ? "+" : "";
    const row = [
      d.label.padEnd(labelWidth),
      fmt(d.baseline).padStart(numWidth),
      fmt(d.virgil).padStart(numWidth),
      `${sign}${fmt(d.delta)}`.padStart(numWidth),
      `${sign}${d.deltaPercent.toFixed(1)}%`.padStart(numWidth),
    ].join(" | ");
    console.log(row);
  }

  console.log("=".repeat(78));
}

function fmt(n: number): string {
  if (Number.isInteger(n)) return String(n);
  return n.toFixed(2);
}

export function saveReport(report: BenchmarkReport, outputDir: string): string {
  mkdirSync(outputDir, { recursive: true });

  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  const repoSlug = report.repo.replace("/", "-");
  const filename = `benchmark-${repoSlug}-${report.issue.number}-${ts}.json`;
  const filepath = join(outputDir, filename);

  writeFileSync(filepath, JSON.stringify(report, null, 2));
  console.log(`\nReport saved to: ${filepath}`);
  return filepath;
}

export function savePlanMarkdown(
  run: BenchmarkRun,
  report: BenchmarkReport,
  outputDir: string,
  tsSlug: string,
): string {
  mkdirSync(outputDir, { recursive: true });

  const repoSlug = report.repo.replace("/", "-");
  const filename = `benchmark-${repoSlug}-${report.issue.number}-${tsSlug}-${run.mode}-plan.md`;
  const filepath = join(outputDir, filename);

  const lines = [
    `# Implementation Plan: ${run.mode}`,
    "",
    `- **Repo**: ${report.repo}`,
    `- **Issue**: #${report.issue.number} — ${report.issue.title}`,
    `- **Model**: ${report.model}`,
    `- **Timestamp**: ${report.timestamp}`,
    `- **Score**: ${run.score.weighted}/100`,
    "",
    "---",
    "",
    run.plan.rawText,
  ];

  writeFileSync(filepath, lines.join("\n"));
  console.log(`Plan saved to: ${filepath}`);
  return filepath;
}

export function saveRunLog(
  run: BenchmarkRun,
  report: BenchmarkReport,
  outputDir: string,
  tsSlug: string,
): string {
  mkdirSync(outputDir, { recursive: true });

  const repoSlug = report.repo.replace("/", "-");
  const filename = `benchmark-${repoSlug}-${report.issue.number}-${tsSlug}-${run.mode}.log.md`;
  const filepath = join(outputDir, filename);

  const lines: string[] = [];

  lines.push(`# Benchmark Log: ${run.mode}`);
  lines.push("");
  lines.push("## Config");
  lines.push("");
  lines.push(`- **Repo**: ${report.repo}`);
  lines.push(`- **Issue**: #${report.issue.number} — ${report.issue.title}`);
  lines.push(`- **Model**: ${report.model}`);
  lines.push(`- **Judge Model**: ${report.judgeModel}`);
  lines.push(`- **Timeout**: ${report.config.timeout}ms`);
  lines.push("");

  // Prompt sent
  lines.push("## Prompt Sent");
  lines.push("");
  lines.push("```");
  lines.push(run.prompt);
  lines.push("```");
  lines.push("");

  // Conversation
  lines.push("## Conversation");
  lines.push("");
  for (const entry of run.messages) {
    lines.push(...formatLogEntry(entry));
  }

  // Scoring section
  lines.push("## Scoring");
  lines.push("");
  lines.push("### Prompt Sent");
  lines.push("");
  lines.push("```");
  lines.push(run.scoringPrompt);
  lines.push("```");
  lines.push("");

  lines.push("### Response");
  lines.push("");
  for (const entry of run.scoringMessages) {
    lines.push(...formatLogEntry(entry));
  }

  writeFileSync(filepath, lines.join("\n"));
  console.log(`Log saved to: ${filepath}`);
  return filepath;
}

function formatLogEntry(entry: LogEntry): string[] {
  const lines: string[] = [];

  // Header with role and optional token/cost info
  let header = `### [${entry.role}]`;
  if (entry.tokens) {
    header += ` (tokens: ${entry.tokens.input ?? "?"}/${entry.tokens.output ?? "?"}`;
    if (entry.cost != null) {
      header += `, cost: $${entry.cost.toFixed(4)}`;
    }
    header += ")";
  }
  lines.push(header);
  lines.push("");

  // Text content
  if (entry.content) {
    lines.push(entry.content);
    lines.push("");
  }

  // Tool calls
  if (entry.toolCalls && entry.toolCalls.length > 0) {
    for (const tc of entry.toolCalls) {
      lines.push(`#### Tool: ${tc.tool}`);
      lines.push("");
      if (tc.input !== undefined) {
        const inputStr = typeof tc.input === "string" ? tc.input : JSON.stringify(tc.input, null, 2);
        lines.push("**Input:**");
        lines.push("```json");
        lines.push(inputStr.length > 1000 ? inputStr.slice(0, 1000) + "\n... (truncated)" : inputStr);
        lines.push("```");
        lines.push("");
      }
      if (tc.output !== undefined) {
        lines.push("**Output:**");
        lines.push("```");
        lines.push(tc.output.length > 500 ? tc.output.slice(0, 500) + "\n... (truncated)" : tc.output);
        lines.push("```");
        lines.push("");
      }
    }
  }

  return lines;
}
