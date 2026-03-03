import type { GitHubIssue } from "./types";

interface GhIssue {
  number: number;
  title: string;
  body: string;
  labels: { name: string }[];
  url: string;
  createdAt: string;
}

export async function fetchIssues(repo: string, limit = 30): Promise<GitHubIssue[]> {
  const proc = Bun.spawn(
    [
      "gh",
      "issue",
      "list",
      "--repo",
      repo,
      "--state",
      "open",
      "--limit",
      String(limit),
      "--json",
      "number,title,body,labels,url,createdAt",
    ],
    { stdout: "pipe", stderr: "pipe" },
  );

  const exitCode = await proc.exited;
  const stdout = await new Response(proc.stdout).text();

  if (exitCode !== 0) {
    const stderr = await new Response(proc.stderr).text();
    console.error(`Failed to fetch issues: ${stderr}`);
    process.exit(1);
  }

  const raw: GhIssue[] = JSON.parse(stdout);
  return raw.map((i) => ({
    number: i.number,
    title: i.title,
    body: i.body ?? "",
    labels: i.labels.map((l) => l.name),
    url: i.url,
    createdAt: i.createdAt,
  }));
}

export function selectIssue(
  issues: GitHubIssue[],
  target: number | "random",
): GitHubIssue {
  if (issues.length === 0) {
    console.error("No open issues found in this repository.");
    process.exit(1);
  }

  if (target !== "random") {
    const found = issues.find((i) => i.number === target);
    if (!found) {
      console.error(`Issue #${target} not found among open issues.`);
      process.exit(1);
    }
    return found;
  }

  // Prefer issues with substantial body text
  const withBody = issues.filter((i) => i.body.length > 100);
  const pool = withBody.length > 0 ? withBody : issues;
  const index = Math.floor(Math.random() * pool.length);
  return pool[index];
}
