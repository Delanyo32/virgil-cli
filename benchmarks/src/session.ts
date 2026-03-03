import { createOpencode } from "@opencode-ai/sdk";
import type { Message, Part, ToolPart, TextPart } from "@opencode-ai/sdk";
import type { RunMetrics, AgentPlan, LogEntry, LogToolCall } from "./types";

type OpenCodeClient = Awaited<ReturnType<typeof createOpencode>>["client"];

let cachedClient: OpenCodeClient | null = null;
let cachedServer: { url: string; close(): void } | null = null;

export async function getOpenCodeClient(): Promise<OpenCodeClient> {
  if (cachedClient) return cachedClient;

  console.log("Starting OpenCode server...");
  const { client, server } = await createOpencode();
  console.log(`OpenCode server running at ${server.url}`);
  cachedClient = client;
  cachedServer = server;
  return client;
}

export async function shutdownClient(): Promise<void> {
  if (cachedServer) {
    cachedServer.close();
    cachedClient = null;
    cachedServer = null;
  }
}

export interface SessionResult {
  plan: AgentPlan;
  metrics: RunMetrics;
  messages: LogEntry[];
  prompt: string;
}

const POLL_INTERVAL_MS = 2000;

export async function runSession(
  client: OpenCodeClient,
  prompt: string,
  model: string,
  timeoutMs: number,
  directory: string,
): Promise<SessionResult> {
  const [providerID, modelID] = parseModel(model);

  // Create session scoped to the target repo directory
  const sessionResult = await client.session.create({
    body: { title: "virgil-benchmark" },
    query: { directory },
  });
  if (sessionResult.error) {
    throw new Error(`Failed to create session: ${JSON.stringify(sessionResult.error)}`);
  }
  const sessionId = sessionResult.data!.id;
  console.log(`  Session created: ${sessionId}`);
  console.log(`  Directory: ${directory}`);
  console.log(`  Model: ${providerID}/${modelID}`);

  const startTime = Date.now();

  // Use promptAsync which starts the session if needed and returns immediately
  console.log(`  Sending prompt (${prompt.length} chars)...`);
  const promptResult = await client.session.promptAsync({
    path: { id: sessionId },
    body: {
      model: { providerID, modelID },
      parts: [{ type: "text", text: prompt }],
    },
    query: { directory },
  });

  if (promptResult.error) {
    console.error(`  Prompt error: ${JSON.stringify(promptResult.error, null, 2)}`);
    throw new Error(`Prompt failed: ${JSON.stringify(promptResult.error)}`);
  }
  console.log(`  Prompt submitted, polling for completion...`);

  // Poll until the session completes
  await waitForSessionIdle(client, sessionId, directory, startTime, timeoutMs);

  const wallClockMs = Date.now() - startTime;
  console.log(`  Session completed in ${(wallClockMs / 1000).toFixed(1)}s`);

  // Fetch all messages from the session
  const messages = await fetchMessages(client, sessionId, directory);
  console.log(`  Messages: ${messages.length}`);
  for (const msg of messages) {
    console.log(`    msg role=${msg.info.role} id=${msg.info.id} parts=${msg.parts.length}`);
    if (msg.info.role === "assistant") {
      const aInfo = msg.info as import("@opencode-ai/sdk").AssistantMessage;
      console.log(`    tokens=${JSON.stringify(aInfo.tokens)} cost=${aInfo.cost} error=${JSON.stringify(aInfo.error)}`);
    }
  }

  const plan = extractPlan(messages);
  const metrics = extractMetrics(messages, wallClockMs);
  const logEntries = toLogEntries(messages);

  console.log(`  Plan length: ${plan.rawText.length} chars`);
  console.log(`  Tokens: ${metrics.totalTokens} | Tool calls: ${metrics.toolCalls} | Files read: ${metrics.filesRead}`);

  return { plan, metrics, messages: logEntries, prompt };
}

async function waitForSessionIdle(
  client: OpenCodeClient,
  sessionId: string,
  directory: string,
  startTime: number,
  timeoutMs: number,
): Promise<void> {
  let pollCount = 0;
  while (true) {
    const elapsed = Date.now() - startTime;
    if (elapsed >= timeoutMs) {
      throw new Error(`Session timed out after ${timeoutMs}ms`);
    }

    await sleep(POLL_INTERVAL_MS);
    pollCount++;

    // Try session.status() first — returns a map of session statuses
    try {
      const statusResult = await client.session.status({
        query: { directory },
      });
      if (statusResult.error) {
        console.log(`  [poll ${pollCount}] status error: ${JSON.stringify(statusResult.error)}`);
      } else if (statusResult.data) {
        const sessionStatus = statusResult.data[sessionId];
        if (sessionStatus) {
          const statusType =
            typeof sessionStatus === "string"
              ? sessionStatus
              : sessionStatus.type;
          if (pollCount <= 5 || pollCount % 10 === 0) {
            console.log(`  [poll ${pollCount}] status=${statusType} (${(elapsed / 1000).toFixed(0)}s)`);
          }
          if (statusType === "idle") return;
          continue;
        } else {
          // Session not found in status map — dump all session IDs
          const allIds = Object.keys(statusResult.data);
          console.log(`  [poll ${pollCount}] session ${sessionId} not in status map. Known sessions: ${allIds.join(", ") || "(none)"}`);
        }
      } else {
        console.log(`  [poll ${pollCount}] status returned no data`);
      }
    } catch (err) {
      console.log(`  [poll ${pollCount}] status() threw: ${err}`);
    }

    // Fallback: poll messages to check if an assistant response exists
    const messages = await fetchMessages(client, sessionId, directory);
    if (pollCount <= 5 || pollCount % 10 === 0) {
      console.log(`  [poll ${pollCount}] messages=${messages.length} roles=[${messages.map(m => m.info.role).join(",")}]`);
      // Dump last message details
      const lastMsg = messages[messages.length - 1];
      if (lastMsg?.info.role === "assistant") {
        const aInfo = lastMsg.info as import("@opencode-ai/sdk").AssistantMessage;
        console.log(`  [poll ${pollCount}] assistant: finish=${aInfo.finish} completed=${aInfo.time?.completed} tokens=${JSON.stringify(aInfo.tokens)} error=${JSON.stringify(aInfo.error)} parts=${lastMsg.parts.length}`);
      }
    }

    const lastMsg = messages[messages.length - 1];
    if (lastMsg?.info.role === "assistant") {
      const aInfo = lastMsg.info as import("@opencode-ai/sdk").AssistantMessage;
      if (aInfo.time?.completed || aInfo.finish || (aInfo.tokens && aInfo.tokens.output > 0)) {
        console.log(`  Assistant response detected (${(elapsed / 1000).toFixed(1)}s)`);
        return;
      }
    }
  }
}

/**
 * Wait for the session to produce an assistant response.
 * Polls session.status() and falls back to message polling.
 */
export async function waitForAssistantResponse(
  client: OpenCodeClient,
  sessionId: string,
  directory: string,
  timeoutMs: number,
): Promise<void> {
  const startTime = Date.now();
  await waitForSessionIdle(client, sessionId, directory, startTime, timeoutMs);
}

async function fetchMessages(
  client: OpenCodeClient,
  sessionId: string,
  directory: string,
): Promise<MessageEntry[]> {
  const messagesResult = await client.session.messages({
    path: { id: sessionId },
    query: { directory },
  });
  if (messagesResult.error) {
    throw new Error(`Failed to fetch messages: ${JSON.stringify(messagesResult.error)}`);
  }
  return (messagesResult.data ?? []) as MessageEntry[];
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function parseModel(model: string): [string, string] {
  const slash = model.indexOf("/");
  if (slash === -1) {
    return ["anthropic", model];
  }
  return [model.slice(0, slash), model.slice(slash + 1)];
}

function rejectAfter(ms: number): Promise<never> {
  return new Promise((_, reject) =>
    setTimeout(() => reject(new Error(`Session timed out after ${ms}ms`)), ms),
  );
}

export interface MessageEntry {
  info: Message;
  parts: Part[];
}

function extractPlan(messages: MessageEntry[]): AgentPlan {
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.info.role === "assistant") {
      const textParts = msg.parts
        .filter((p): p is TextPart => p.type === "text")
        .map((p) => p.text)
        .join("\n");
      if (textParts.length > 0) {
        return { rawText: textParts };
      }
    }
  }
  return { rawText: "(no plan extracted)" };
}

function extractMetrics(messages: MessageEntry[], wallClockMs: number): RunMetrics {
  let promptTokens = 0;
  let completionTokens = 0;
  let costUsd = 0;
  let toolCalls = 0;
  let globCalls = 0;
  let bashCalls = 0;
  let messageRounds = 0;
  const filesRead = new Set<string>();
  const toolBreakdown: Record<string, number> = {};

  for (const msg of messages) {
    if (msg.info.role === "assistant") {
      messageRounds++;
      const aInfo = msg.info as import("@opencode-ai/sdk").AssistantMessage;
      if (aInfo.tokens) {
        promptTokens += aInfo.tokens.input ?? 0;
        completionTokens += aInfo.tokens.output ?? 0;
      }
      costUsd += aInfo.cost ?? 0;
    }

    for (const part of msg.parts) {
      if (part.type === "tool") {
        toolCalls++;
        const toolPart = part as ToolPart;
        const toolName = toolPart.tool;
        toolBreakdown[toolName] = (toolBreakdown[toolName] ?? 0) + 1;

        const toolLower = toolName.toLowerCase();
        if (toolLower.includes("glob") || toolLower.includes("find")) {
          globCalls++;
        }
        if (toolLower.includes("bash") || toolLower.includes("shell") || toolLower.includes("exec")) {
          bashCalls++;
        }

        const input = toolPart.state && "input" in toolPart.state ? toolPart.state.input : undefined;
        if (input) {
          if (toolLower.includes("read") || toolLower.includes("file")) {
            const path = input.path ?? input.file_path ?? input.filePath ?? input.filename;
            if (typeof path === "string") {
              filesRead.add(path);
            }
          }
        }
      }
    }
  }

  console.log(`  Tool breakdown: ${Object.entries(toolBreakdown).map(([k, v]) => `${k}(${v})`).join(", ")}`);

  return {
    promptTokens,
    completionTokens,
    totalTokens: promptTokens + completionTokens,
    costUsd,
    wallClockMs,
    filesRead: filesRead.size,
    globCalls,
    bashCalls,
    toolCalls,
    toolBreakdown,
    messageRounds,
  };
}

export function toLogEntries(messages: MessageEntry[]): LogEntry[] {
  return messages.map((msg) => {
    const entry: LogEntry = {
      role: msg.info.role,
      content: "",
    };

    // Extract text content
    const textParts = msg.parts
      .filter((p): p is TextPart => p.type === "text")
      .map((p) => p.text);
    entry.content = textParts.join("\n");

    // Extract tool calls
    const toolParts = msg.parts.filter((p): p is ToolPart => p.type === "tool");
    if (toolParts.length > 0) {
      entry.toolCalls = toolParts.map((tp) => {
        const tc: LogToolCall = { tool: tp.tool };
        if (tp.state && "input" in tp.state) {
          tc.input = tp.state.input;
        }
        if (tp.state && "output" in tp.state) {
          const out = tp.state.output;
          tc.output = typeof out === "string" ? out : JSON.stringify(out);
        }
        return tc;
      });
    }

    // Extract tokens and cost from assistant messages
    if (msg.info.role === "assistant") {
      const aInfo = msg.info as import("@opencode-ai/sdk").AssistantMessage;
      if (aInfo.tokens) {
        entry.tokens = {
          input: aInfo.tokens.input ?? undefined,
          output: aInfo.tokens.output ?? undefined,
        };
      }
      if (aInfo.cost != null) {
        entry.cost = aInfo.cost;
      }
      if (aInfo.time?.completed) {
        entry.timestamp = new Date(aInfo.time.completed).toISOString();
      }
    }

    return entry;
  });
}
