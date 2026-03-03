export type BenchmarkMode = "baseline" | "virgil" | "both";

export interface BenchmarkConfig {
  repo: string;
  issue: number | "random";
  mode: BenchmarkMode;
  model: string;
  judgeModel: string;
  timeout: number;
  output: string;
  virgilBin: string;
  dataDir?: string;
}

export interface FileConfig {
  repo?: string;
  issue?: number | string;
  mode?: string;
  model?: string;
  judgeModel?: string;
  timeout?: number;
  output?: string;
  virgilBin?: string;
}

export interface GitHubIssue {
  number: number;
  title: string;
  body: string;
  labels: string[];
  url: string;
  createdAt: string;
}

export interface RunMetrics {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  costUsd: number;
  wallClockMs: number;
  filesRead: number;
  globCalls: number;
  bashCalls: number;
  toolCalls: number;
  toolBreakdown: Record<string, number>;
  messageRounds: number;
}

export interface ScoreDimensions {
  completeness: number;
  accuracy: number;
  specificity: number;
  feasibility: number;
  fileIdentification: number;
}

export interface ScoreResult {
  dimensions: ScoreDimensions;
  weighted: number;
  rationale: string;
}

export interface AgentPlan {
  rawText: string;
}

export interface LogToolCall {
  tool: string;
  input?: unknown;
  output?: string;
}

export interface LogEntry {
  role: string;
  timestamp?: string;
  content: string;
  toolCalls?: LogToolCall[];
  tokens?: { input?: number; output?: number };
  cost?: number;
}

export interface BenchmarkRun {
  mode: "baseline" | "virgil";
  plan: AgentPlan;
  metrics: RunMetrics;
  score: ScoreResult;
  messages: LogEntry[];
  scoringMessages: LogEntry[];
  prompt: string;
  scoringPrompt: string;
}

export interface MetricDelta {
  label: string;
  baseline: number;
  virgil: number;
  delta: number;
  deltaPercent: number;
}

export interface BenchmarkReport {
  repo: string;
  issue: GitHubIssue;
  model: string;
  judgeModel: string;
  timestamp: string;
  config: BenchmarkConfig;
  baseline?: BenchmarkRun;
  virgil?: BenchmarkRun;
  deltas?: MetricDelta[];
}
