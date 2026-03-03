import type { BenchmarkRun, MetricDelta } from "./types";

function delta(label: string, baseline: number, virgil: number): MetricDelta {
  const d = virgil - baseline;
  const pct = baseline !== 0 ? (d / baseline) * 100 : virgil !== 0 ? 100 : 0;
  return {
    label,
    baseline,
    virgil,
    delta: Math.round(d * 100) / 100,
    deltaPercent: Math.round(pct * 100) / 100,
  };
}

export function computeDeltas(baseline: BenchmarkRun, virgil: BenchmarkRun): MetricDelta[] {
  return [
    // Scores
    delta("Weighted Score", baseline.score.weighted, virgil.score.weighted),
    delta("Completeness", baseline.score.dimensions.completeness, virgil.score.dimensions.completeness),
    delta("Accuracy", baseline.score.dimensions.accuracy, virgil.score.dimensions.accuracy),
    delta("Specificity", baseline.score.dimensions.specificity, virgil.score.dimensions.specificity),
    delta("Feasibility", baseline.score.dimensions.feasibility, virgil.score.dimensions.feasibility),
    delta("File Identification", baseline.score.dimensions.fileIdentification, virgil.score.dimensions.fileIdentification),
    // Efficiency
    delta("Total Tokens", baseline.metrics.totalTokens, virgil.metrics.totalTokens),
    delta("Cost (USD)", baseline.metrics.costUsd, virgil.metrics.costUsd),
    delta("Wall Clock (ms)", baseline.metrics.wallClockMs, virgil.metrics.wallClockMs),
    delta("Files Read", baseline.metrics.filesRead, virgil.metrics.filesRead),
    delta("Glob Calls", baseline.metrics.globCalls, virgil.metrics.globCalls),
    delta("Bash Calls", baseline.metrics.bashCalls, virgil.metrics.bashCalls),
    delta("Tool Calls", baseline.metrics.toolCalls, virgil.metrics.toolCalls),
    delta("Message Rounds", baseline.metrics.messageRounds, virgil.metrics.messageRounds),
  ];
}
