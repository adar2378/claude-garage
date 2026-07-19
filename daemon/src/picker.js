import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

// D-picker: browsers deliberately never expose absolute filesystem paths,
// so the "add workspace" folder picker has to run daemon-side. macOS-only —
// non-darwin hosts keep the manual path-entry fallback in the UI.
const PICKER_SCRIPT =
  'POSIX path of (choose folder with prompt "Add claude-garage workspace")';

// osascript blocks for as long as the dialog is open (the user may leave it
// sitting there) — give it a generous timeout rather than the execFile
// default, which would otherwise fail an interaction that's still in progress.
const PICKER_TIMEOUT_MS = 120_000;

export default async function pickerRoutes(app) {
  app.post("/api/pick-directory", async (req, reply) => {
    if (process.platform !== "darwin") {
      return reply
        .code(501)
        .send({ error: "directory picker is only available on macOS" });
    }

    let stdout;
    try {
      ({ stdout } = await run("osascript", ["-e", PICKER_SCRIPT], {
        timeout: PICKER_TIMEOUT_MS,
      }));
    } catch (err) {
      // osascript exits 1 with "User canceled" in stderr when the user
      // dismisses the dialog — that's a normal outcome, not an error.
      const stderr = err?.stderr ?? "";
      if (stderr.includes("User canceled")) {
        return reply.code(200).send({ cancelled: true });
      }
      return reply.code(500).send({ error: err?.message ?? "osascript failed" });
    }

    // osascript's `POSIX path of` always trails with a newline, and folder
    // paths additionally trail with "/" (except the root "/" itself).
    let dir = stdout.trimEnd();
    if (dir.length > 1 && dir.endsWith("/")) {
      dir = dir.slice(0, -1);
    }
    return reply.code(200).send({ dir });
  });
}
