import { readFile } from "node:fs/promises";

import type {
  ContextEvent,
  EventName,
  RunContext,
} from "./types.gen.js";

/** Where the run context may come from. All optional; defaults read the process. */
export interface ReadContextOptions {
  env?: Record<string, string | undefined>;
  /** Read all of standard input. Default: the process's stdin. */
  stdin?: () => Promise<string>;
}

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

/**
 * The run context: `GARAGE_CONTEXT`, else the file `GARAGE_CONTEXT_FILE`
 * names, else standard input.
 */
export async function readContext(
  options: ReadContextOptions = {},
): Promise<Context> {
  const env = options.env ?? process.env;
  let raw: string;
  const inline = env["GARAGE_CONTEXT"];
  const file = env["GARAGE_CONTEXT_FILE"];
  if (inline) {
    raw = inline;
  } else if (file) {
    raw = await readFile(file, "utf8");
  } else {
    raw = await (options.stdin ?? readStdin)();
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    throw new Error(
      `cannot read the run context: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  return new Context(parsed as RunContext);
}

/**
 * A typed view of the run context. Accessors return `undefined` for a `null`
 * or empty field. `raw` is the whole context.
 */
export class Context {
  readonly raw: RunContext;

  constructor(raw: RunContext) {
    this.raw = raw;
  }

  /** The focused workspace. */
  get workspace(): string | undefined {
    return this.raw.focus?.workspace || undefined;
  }

  /** The focused session id (`garage/<workspace>/<label>`). */
  get session(): string | undefined {
    return this.raw.focus?.session?.id || undefined;
  }

  /** Why this run started: `key`, `button`, `widget`, `palette`, `event`, `timer`, `pane` or `cli`. */
  get triggerKind(): string {
    return this.raw.trigger?.kind ?? "";
  }

  /** The `args` of the button or widget click that started the run. */
  get args(): unknown {
    return this.raw.trigger?.args;
  }

  /** The event of an event hook run, else `undefined`. */
  get event(): ContextEvent | undefined {
    return this.raw.event ?? undefined;
  }

  /** The name of the event, if this run is an event hook. */
  get eventName(): EventName | undefined {
    return this.event?.name as EventName | undefined;
  }

  /** The event payload (`{}` for `daemon.started` and `timer`). */
  get eventData(): unknown {
    return this.event?.data;
  }

  /** The words after `claude-garage` of a CLI run. */
  get argv(): string[] {
    const args = this.raw.trigger?.args;
    if (args !== null && typeof args === "object" && "argv" in args) {
      const argv = (args as { argv: unknown }).argv;
      if (Array.isArray(argv)) {
        return argv.filter((a): a is string => typeof a === "string");
      }
    }
    return [];
  }

  /** The plugin's own writable folder. */
  get stateDir(): string | undefined {
    return this.raw.plugin?.state_dir || undefined;
  }

  /** The directory of the focused session (the caller's directory for a CLI run). */
  get cwd(): string | undefined {
    const focus = this.raw.focus as { dir?: string | null } | undefined;
    return this.raw.focus?.session?.dir || focus?.dir || undefined;
  }
}
