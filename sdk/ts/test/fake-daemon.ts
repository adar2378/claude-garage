import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

export interface Seen {
  method: string;
  path: string;
  authorization: string | undefined;
  contentType: string | undefined;
  body: unknown;
}

export type Reply = [status: number, body: string];

/** A local HTTP server that records every request and answers from a function. */
export class FakeDaemon {
  readonly seen: Seen[] = [];
  readonly #server: Server;
  base = "";

  constructor(reply: (req: Seen) => Reply) {
    this.#server = createServer((req, res) => {
      const chunks: Buffer[] = [];
      req.on("data", (c: Buffer) => chunks.push(c));
      req.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        const seen: Seen = {
          method: req.method ?? "",
          path: req.url ?? "",
          authorization: req.headers["authorization"],
          contentType: req.headers["content-type"],
          body: text === "" ? undefined : JSON.parse(text),
        };
        this.seen.push(seen);
        const [status, body] = reply(seen);
        res.writeHead(status, { "content-type": "application/json" });
        res.end(body);
      });
    });
  }

  async start(): Promise<this> {
    await new Promise<void>((ok) =>
      this.#server.listen(0, "127.0.0.1", () => ok()),
    );
    const { port } = this.#server.address() as AddressInfo;
    this.base = `http://127.0.0.1:${port}`;
    return this;
  }

  async stop(): Promise<void> {
    await new Promise<void>((ok) => this.#server.close(() => ok()));
  }
}
