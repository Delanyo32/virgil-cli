import { parseConfig, printConfig } from "./config";
import { checkPrerequisites } from "./prerequisites";
import { ensureRepo } from "./repo";
import { fetchIssues, selectIssue } from "./issues";
import { runBaseline, runVirgilEnhanced } from "./runner";
import { computeDeltas } from "./metrics";
import { printRunSummary, printComparisonTable, saveReport, savePlanMarkdown, saveRunLog } from "./report";
import { shutdownClient } from "./session";
import type { BenchmarkReport, BenchmarkRun } from "./types";

async function main() {
  const config = parseConfig();
  printConfig(config);

  // Step 1: Prerequisites
  await checkPrerequisites(config.virgilBin);

  // Step 2: Clone/refresh repo
  const repoPath = await ensureRepo(config.repo);

  // Step 2.5: Pre-parse codebase with virgil-cli
  if (config.mode === "virgil" || config.mode === "both") {
    const dataDir = repoPath + "-data";
    console.log(`\nPre-parsing codebase with virgil-cli...`);
    const proc = Bun.spawn([config.virgilBin, "parse", repoPath, "--output", dataDir], {
      stdout: "pipe",
      stderr: "pipe",
    });
    const exitCode = await proc.exited;
    if (exitCode !== 0) {
      const stderr = await new Response(proc.stderr).text();
      console.error(`virgil-cli parse failed (exit ${exitCode}): ${stderr}`);
      process.exit(1);
    }
    console.log("Pre-parse complete.");
    config.dataDir = dataDir;
  }

  // Step 3: Select issue
  console.log("Fetching issues...");
  const issues = await fetchIssues(config.repo);
  const issue = selectIssue(issues, config.issue);
  console.log(`Selected issue #${issue.number}: ${issue.title}`);

  // Step 4+5: Run benchmarks
  let baseline: BenchmarkRun | undefined;
  let virgil: BenchmarkRun | undefined;

  try {
    if (config.mode === "baseline" || config.mode === "both") {
      try {
        baseline = await runBaseline(config, issue, repoPath);
        printRunSummary(baseline);
      } catch (err) {
        console.error(`\nBaseline run failed: ${err}`);
      }
    }

    if (config.mode === "virgil" || config.mode === "both") {
      try {
        virgil = await runVirgilEnhanced(config, issue, repoPath);
        printRunSummary(virgil);
      } catch (err) {
        console.error(`\nVirgil-enhanced run failed: ${err}`);
      }
    }

    // Step 6: Compare if both ran
    const deltas =
      baseline && virgil ? computeDeltas(baseline, virgil) : undefined;
    if (deltas) {
      printComparisonTable(deltas);
    }

    // Step 7: Save report and output files
    if (baseline || virgil) {
      const report: BenchmarkReport = {
        repo: config.repo,
        issue,
        model: config.model,
        judgeModel: config.judgeModel,
        timestamp: new Date().toISOString(),
        config,
        baseline,
        virgil,
        deltas,
      };

      saveReport(report, config.output);

      // Use a shared timestamp slug for all output files
      const tsSlug = new Date().toISOString().replace(/[:.]/g, "-");

      if (baseline) {
        savePlanMarkdown(baseline, report, config.output, tsSlug);
        saveRunLog(baseline, report, config.output, tsSlug);
      }
      if (virgil) {
        savePlanMarkdown(virgil, report, config.output, tsSlug);
        saveRunLog(virgil, report, config.output, tsSlug);
      }
    } else {
      console.error("\nBoth runs failed — no report saved.");
      process.exit(1);
    }
  } finally {
    await shutdownClient();
  }
}

main().catch((err) => {
  console.error("Benchmark failed:", err);
  process.exit(1);
});
