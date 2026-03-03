import type { Message, Part, TextPart } from "@opencode-ai/sdk";
import type { GitHubIssue, ScoreResult, ScoreDimensions, LogEntry } from "./types";
import { buildScoringPrompt } from "./prompts";
import { getOpenCodeClient, waitForAssistantResponse, toLogEntries } from "./session";
import type { MessageEntry } from "./session";

const WEIGHTS: Record<keyof ScoreDimensions, number> = {
  completeness: 0.25,
  accuracy: 0.25,
  specificity: 0.2,
  feasibility: 0.15,
  fileIdentification: 0.15,
};

function computeWeighted(dims: ScoreDimensions): number {
  let total = 0;
  for (const [key, weight] of Object.entries(WEIGHTS)) {
    total += dims[key as keyof ScoreDimensions] * weight;
  }
  return Math.round(total * 100) / 100;
}

const SCORING_TIMEOUT_MS = 120_000;

export interface ScorePlanResult {
  score: ScoreResult;
  messages: LogEntry[];
  prompt: string;
}

export async function scorePlan(
  issue: GitHubIssue,
  planText: string,
  mode: "baseline" | "virgil",
  model: string,
  directory: string,
): Promise<ScorePlanResult> {
  const client = await getOpenCodeClient();
  const prompt = buildScoringPrompt(issue, planText, mode);

  const [providerID, modelID] = parseModel(model);

  const sessionResult = await client.session.create({
    body: { title: `virgil-benchmark-scoring-${mode}` },
    query: { directory },
  });
  if (sessionResult.error) {
    throw new Error(`Failed to create scoring session: ${JSON.stringify(sessionResult.error)}`);
  }
  const sessionId = sessionResult.data!.id;

  console.log(`  Scoring session: ${sessionId}`);

  const promptResult = await client.session.promptAsync({
    path: { id: sessionId },
    body: {
      model: { providerID, modelID },
      parts: [{ type: "text", text: prompt }],
    },
    query: { directory },
  });

  if (promptResult.error) {
    console.error(`  Scoring prompt error: ${JSON.stringify(promptResult.error)}`);
    throw new Error(`Scoring prompt failed: ${JSON.stringify(promptResult.error)}`);
  }

  console.log(`  Waiting for scoring model to complete...`);
  await waitForAssistantResponse(client, sessionId, directory, SCORING_TIMEOUT_MS);

  // Fetch messages and extract JSON score
  const messages = await fetchScoringMessages(client, sessionId, directory);
  const logEntries = toLogEntries(messages as MessageEntry[]);

  console.log(`  Scoring messages: ${messages.length}`);

  for (let i = messages.length - 1; i >= 0; i--) {
    const msg: { info: Message; parts: Part[] } = messages[i];
    if (msg.info.role === "assistant") {
      for (const part of msg.parts) {
        if (part.type === "text") {
          const textPart = part as TextPart;
          const parsed = tryParseJson(textPart.text);
          if (parsed) {
            const dims: ScoreDimensions = {
              completeness: Number(parsed.completeness) || 0,
              accuracy: Number(parsed.accuracy) || 0,
              specificity: Number(parsed.specificity) || 0,
              feasibility: Number(parsed.feasibility) || 0,
              fileIdentification: Number(parsed.fileIdentification) || 0,
            };
            console.log(`  Score: ${computeWeighted(dims)} (weighted)`);
            return {
              score: {
                dimensions: dims,
                weighted: computeWeighted(dims),
                rationale: String(parsed.rationale ?? ""),
              },
              messages: logEntries,
              prompt,
            };
          }
        }
      }
    }
  }

  // Debug: dump what we actually got
  console.warn(`Warning: Could not parse scoring response for ${mode} run.`);
  for (const msg of messages) {
    if (msg.info.role === "assistant") {
      for (const part of msg.parts) {
        if (part.type === "text") {
          console.warn(`  Response text: ${(part as TextPart).text.slice(0, 500)}`);
        }
      }
    }
  }

  const zeroDims: ScoreDimensions = {
    completeness: 0,
    accuracy: 0,
    specificity: 0,
    feasibility: 0,
    fileIdentification: 0,
  };
  return {
    score: { dimensions: zeroDims, weighted: 0, rationale: "Failed to parse scoring response" },
    messages: logEntries,
    prompt,
  };
}

async function fetchScoringMessages(
  client: Awaited<ReturnType<typeof getOpenCodeClient>>,
  sessionId: string,
  directory: string,
) {
  const messagesResult = await client.session.messages({
    path: { id: sessionId },
    query: { directory },
  });
  if (messagesResult.error) {
    throw new Error(`Failed to fetch scoring messages: ${JSON.stringify(messagesResult.error)}`);
  }
  return messagesResult.data ?? [];
}

function tryParseJson(text: string): Record<string, unknown> | null {
  // Try direct parse
  try {
    return JSON.parse(text);
  } catch {
    // Try extracting from markdown code block (```json ... ``` or ``` ... ```)
    const codeBlockMatch = text.match(/```(?:json)?\s*\n?([\s\S]*?)```/);
    if (codeBlockMatch) {
      try {
        return JSON.parse(codeBlockMatch[1].trim());
      } catch {
        // fall through
      }
    }

    // Try extracting a JSON object containing "completeness" from surrounding text
    const match = text.match(/\{[^{}]*"completeness"[^{}]*\}/);
    if (match) {
      try {
        return JSON.parse(match[0]);
      } catch {
        return null;
      }
    }
    return null;
  }
}

function parseModel(model: string): [string, string] {
  const slash = model.indexOf("/");
  if (slash === -1) return ["anthropic", model];
  return [model.slice(0, slash), model.slice(slash + 1)];
}
