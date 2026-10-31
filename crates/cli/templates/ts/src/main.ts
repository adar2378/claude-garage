import { daemonFromEnv, greeting, pushWidget } from "./plugin.js";

// The run context is JSON in GARAGE_CONTEXT (or the file GARAGE_CONTEXT_FILE when it is large).
async function readContext(): Promise<any> {
  if (process.env.GARAGE_CONTEXT) return JSON.parse(process.env.GARAGE_CONTEXT);
  if (process.env.GARAGE_CONTEXT_FILE) {
    const { readFile } = await import("node:fs/promises");
    return JSON.parse(await readFile(process.env.GARAGE_CONTEXT_FILE, "utf8"));
  }
  return {};
}

try {
  const ctx = await readContext();
  await pushWidget(daemonFromEnv(process.env), "status", greeting(ctx?.focus?.workspace || undefined));
} catch (e) {
  console.error(`@@ID@@: ${e instanceof Error ? e.message : e}`);
  process.exit(1);
}
