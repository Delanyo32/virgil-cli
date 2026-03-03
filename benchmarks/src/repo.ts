import { existsSync } from "node:fs";
import { join } from "node:path";

const REPOS_DIR = join(import.meta.dir, "..", "repos");

function repoDir(repo: string): string {
  const safeName = repo.replace("/", "-");
  return join(REPOS_DIR, safeName);
}

async function run(cmd: string[], cwd?: string): Promise<{ ok: boolean; output: string }> {
  const proc = Bun.spawn(cmd, {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  const exitCode = await proc.exited;
  const stdout = await new Response(proc.stdout).text();
  const stderr = await new Response(proc.stderr).text();
  return { ok: exitCode === 0, output: stdout || stderr };
}

export async function ensureRepo(repo: string): Promise<string> {
  const dir = repoDir(repo);

  if (existsSync(join(dir, ".git"))) {
    console.log(`Updating existing clone: ${dir}`);
    const result = await run(["git", "pull", "--ff-only"], dir);
    if (!result.ok) {
      console.warn(`Warning: git pull failed, using existing clone. ${result.output}`);
    }
  } else {
    console.log(`Cloning ${repo} into ${dir}...`);
    const result = await run([
      "git",
      "clone",
      "--depth",
      "1",
      `https://github.com/${repo}.git`,
      dir,
    ]);
    if (!result.ok) {
      console.error(`Failed to clone ${repo}: ${result.output}`);
      process.exit(1);
    }
  }

  console.log(`Repo ready: ${dir}`);
  return dir;
}
