/**
 * @claude-garage/plugin-sdk: a typed client for the claude-garage plugin API.
 *
 * ```ts
 * import { Api, readContext, textWidget } from "@claude-garage/plugin-sdk";
 *
 * const ctx = await readContext();
 * const api = Api.fromEnv();
 * await api.putWidget("bar", textWidget(`hello from ${ctx.workspace ?? "nowhere"}`));
 * ```
 *
 * Zero runtime dependencies. Needs Node 24 or later (global `fetch`).
 */
export { Api, GarageError, textWidget } from "./api.js";
export type { ApiOptions, Target } from "./api.js";
export { Context, readContext } from "./context.js";
export type { ReadContextOptions } from "./context.js";
export type * from "./types.gen.js";
export type { Context as SessionContext } from "./types.gen.js";
