interface ToolCheck {
  name: string;
  command: string[];
  required: boolean;
}

const TOOLS: ToolCheck[] = [
  { name: "git", command: ["git", "--version"], required: true },
  { name: "gh", command: ["gh", "--version"], required: true },
  { name: "opencode", command: ["opencode", "--version"], required: true },
];

async function checkTool(tool: ToolCheck): Promise<{ name: string; ok: boolean; error?: string }> {
  try {
    const proc = Bun.spawn(tool.command, {
      stdout: "pipe",
      stderr: "pipe",
    });
    const exitCode = await proc.exited;
    if (exitCode !== 0) {
      return { name: tool.name, ok: false, error: `exited with code ${exitCode}` };
    }
    return { name: tool.name, ok: true };
  } catch {
    return { name: tool.name, ok: false, error: "not found in PATH" };
  }
}

export async function checkPrerequisites(virgilBin: string): Promise<void> {
  const tools: ToolCheck[] = [
    ...TOOLS,
    { name: "virgil", command: [virgilBin, "--help"], required: true },
  ];

  const results = await Promise.all(tools.map(checkTool));
  const failures = results.filter((r) => !r.ok);

  if (failures.length > 0) {
    console.error("\nMissing prerequisites:");
    for (const f of failures) {
      console.error(`  - ${f.name}: ${f.error}`);
    }
    console.error(
      "\nPlease install missing tools and ensure they are in your PATH.",
    );
    process.exit(1);
  }

  console.log("All prerequisites satisfied.");
}
