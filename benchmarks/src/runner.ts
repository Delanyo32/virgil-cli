import type { BenchmarkConfig, GitHubIssue, BenchmarkRun } from "./types";
import { buildBaselinePrompt, buildVirgilPrompt } from "./prompts";
import { getOpenCodeClient, runSession } from "./session";
import { scorePlan } from "./scorer";

export async function runBaseline(
  config: BenchmarkConfig,
  issue: GitHubIssue,
  repoPath: string,
): Promise<BenchmarkRun> {
  console.log("\n--- Running BASELINE ---");
  console.log(`Issue #${issue.number}: ${issue.title}`);
  console.log(`Model: ${config.model}`);

  const client = await getOpenCodeClient();
  const prompt = buildBaselinePrompt(issue, repoPath);

  const { plan, metrics, messages, prompt: sessionPrompt } = await runSession(
    client,
    prompt,
    config.model,
    config.timeout,
    repoPath,
  );

  console.log("Baseline run complete. Scoring...");
  console.log(`Judge model: ${config.judgeModel}`);
  const { score, messages: scoringMessages, prompt: scoringPrompt } = await scorePlan(
    issue,
    plan.rawText,
    "baseline",
    config.judgeModel,
    repoPath,
  );

  return {
    mode: "baseline",
    plan,
    metrics,
    score,
    messages,
    scoringMessages,
    prompt: sessionPrompt,
    scoringPrompt,
  };
}

export async function runVirgilEnhanced(
  config: BenchmarkConfig,
  issue: GitHubIssue,
  repoPath: string,
): Promise<BenchmarkRun> {
  console.log("\n--- Running VIRGIL-ENHANCED ---");
  console.log(`Issue #${issue.number}: ${issue.title}`);
  console.log(`Model: ${config.model}`);

  const client = await getOpenCodeClient();
  const prompt = buildVirgilPrompt(issue, repoPath, config.virgilBin, config.dataDir);

  const { plan, metrics, messages, prompt: sessionPrompt } = await runSession(
    client,
    prompt,
    config.model,
    config.timeout,
    repoPath,
  );

  console.log("Virgil-enhanced run complete. Scoring...");
  console.log(`Judge model: ${config.judgeModel}`);
  const { score, messages: scoringMessages, prompt: scoringPrompt } = await scorePlan(
    issue,
    plan.rawText,
    "virgil",
    config.judgeModel,
    repoPath,
  );

  return {
    mode: "virgil",
    plan,
    metrics,
    score,
    messages,
    scoringMessages,
    prompt: sessionPrompt,
    scoringPrompt,
  };
}
