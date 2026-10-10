// Native session-host backend — the tmux replacement used on Windows.
//
// Each garage session is run by a detached `garage-wall host` process (see
// wall/src/host.rs for the protocol). A host registers itself in
// <garage dir>/hosts/<id>.json; this module reads that registry for listing
// (no process spawn per poll) and talks to a host's loopback control socket
// to kill or rename a session.
import { spawn } from "node:child_process";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  rmSync,
  unlinkSync,
} from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const PREFIX = "garage/";
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

const F_SHUTDOWN = 4;
const F_RENAME = 5;
const F_TYPE = 6;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function garageDir() {
  return process.env.GARAGE_DIR ?? path.join(os.homedir(), ".garage");
}

function hostsDir() {
  return path.join(garageDir(), "hosts");
}

const stem = (id) => id.replaceAll("/", "__");
const registryFile = (id) => path.join(hostsDir(), `${stem(id)}.json`);

function pidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
}

// Every live host, dropping (and cleaning up) registry files whose host
// process is gone — a crashed host never got to remove its own.
function readHosts() {
  let names;
  try {
    names = readdirSync(hostsDir());
  } catch {
    return [];
  }
  const hosts = [];
  for (const name of names) {
    if (!name.endsWith(".json")) continue;
    const file = path.join(hostsDir(), name);
    let info;
    try {
      info = JSON.parse(readFileSync(file, "utf8"));
    } catch {
      continue; // half-written or foreign file — try again next tick
    }
    if (!info?.id || !pidAlive(info.pid)) {
      try {
        unlinkSync(file);
      } catch {}
      continue;
    }
    hosts.push(info);
  }
  return hosts;
}

function hostBinary() {
  if (process.env.GARAGE_HOST_BIN) return process.env.GARAGE_HOST_BIN;
  const target = `${process.platform}-${process.arch}`;
  const exe = process.platform === "win32" ? ".exe" : "";
  const candidates = [
    path.join(ROOT, "wall", "dist", `garage-wall-${target}${exe}`),
    path.join(ROOT, "wall", "target", "release", `garage-wall${exe}`),
  ];
  const found = candidates.find((c) => existsSync(c));
  if (!found) throw new Error(`garage-wall binary not found (looked in ${candidates.join(", ")})`);
  return found;
}

export async function listSessions() {
  return readHosts()
    .filter((h) => h.id.startsWith(PREFIX))
    .map((h) => {
      const [, workspace, label] = h.id.split("/");
      return {
        id: h.id,
        workspace,
        label,
        dir: h.dir,
        attached: false,
        createdAt: Number.isFinite(h.createdAt) && h.createdAt > 0 ? h.createdAt : null,
      };
    });
}

// id -> { path, title, pid } for every garage session. `pid` is the pid of
// the spawned command (claude), which `claude agents --json` reports.
export async function listPaneInfo() {
  const panes = new Map();
  for (const h of readHosts()) {
    if (!h.id.startsWith(PREFIX)) continue;
    panes.set(h.id, { path: h.dir, title: h.title ?? "", pid: h.childPid });
  }
  return panes;
}

export async function hasSession(id) {
  return readHosts().some((h) => h.id === id);
}

const helloBuf = (info, mode) => {
  const b = Buffer.alloc(3 + 32 + 1 + 4);
  b.write("GH1", 0, "latin1");
  b.write(info.token, 3, "latin1");
  b.writeUInt8(mode.charCodeAt(0), 35);
  return b;
};

const frame = (type, payload = Buffer.alloc(0)) => {
  const head = Buffer.alloc(5);
  head.writeUInt8(type, 0);
  head.writeUInt32LE(payload.length, 1);
  return Buffer.concat([head, payload]);
};

// One request/reply on a host's control socket.
function control(info, type, payload) {
  return new Promise((resolve, reject) => {
    const sock = net.connect({ host: "127.0.0.1", port: info.port });
    const timer = setTimeout(() => {
      sock.destroy();
      reject(new Error("session host did not answer"));
    }, 3000);
    let buf = Buffer.alloc(0);
    sock.on("connect", () => {
      sock.write(helloBuf(info, "Q"));
      sock.write(frame(type, payload));
    });
    sock.on("data", (d) => {
      buf = Buffer.concat([buf, d]);
      if (buf.length >= 4 && buf.length >= 4 + buf.readUInt32LE(0)) {
        clearTimeout(timer);
        sock.destroy();
        resolve(buf.subarray(4, 4 + buf.readUInt32LE(0)));
      }
    });
    sock.on("error", (err) => {
      clearTimeout(timer);
      reject(err);
    });
    sock.on("close", () => clearTimeout(timer));
  });
}

async function findHost(id) {
  const h = readHosts().find((x) => x.id === id);
  if (!h) throw new Error(`no such session: ${id}`);
  return h;
}

// `command`/`extraArgs` follow tmux.js's contract: separate argv entries.
export async function createSession(id, dir, command, extraArgs = []) {
  mkdirSync(hostsDir(), { recursive: true });
  const logPath = path.join(hostsDir(), `${stem(id)}.log`);
  const log = openSync(logPath, "w");
  let child;
  try {
    child = spawn(
      hostBinary(),
      ["host", "--id", id, "--dir", dir, "--", command, ...extraArgs],
      { detached: true, stdio: ["ignore", log, log], windowsHide: true, env: process.env }
    );
  } finally {
    closeSync(log);
  }
  let spawnError = null;
  child.on("error", (err) => {
    spawnError = err;
  });
  let exited = false;
  child.on("exit", () => {
    exited = true;
  });
  child.unref();

  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError;
    if (readHosts().some((h) => h.id === id)) return;
    if (exited) break;
    await sleep(50);
  }
  let detail = "";
  try {
    detail = readFileSync(logPath, "utf8").trim();
  } catch {}
  throw new Error(`session host for ${id} failed to start${detail ? `: ${detail}` : ""}`);
}

const draftFile = (id) => path.join(hostsDir(), `${stem(id)}.draft`);

// The unsent prompt a host saved before it died (power cut, crash), or null.
export function readDraft(id) {
  try {
    const text = readFileSync(draftFile(id), "utf8");
    return text.trim() ? text : null;
  } catch {
    return null;
  }
}

const imagesDir = (id) => path.join(hostsDir(), `${stem(id)}.images`);

// Pasted screenshots saved with the draft (absolute paths, oldest first).
export function readDraftImages(id) {
  try {
    return readdirSync(imagesDir(id))
      .filter((f) => f.endsWith(".png"))
      .sort()
      .map((f) => path.join(imagesDir(id), f).replaceAll("\\", "/"));
  } catch {
    return [];
  }
}

export function dropDraft(id) {
  try {
    unlinkSync(draftFile(id));
  } catch {}
}

export function dropDraftImages(id) {
  try {
    rmSync(imagesDir(id), { recursive: true, force: true });
  } catch {}
}

// Ask a host to paste `text` into its prompt once Claude's input box is up.
export async function typeDraft(id, text) {
  const h = await findHost(id);
  await control(h, F_TYPE, Buffer.from(text, "utf8"));
}

export async function killSession(id) {
  dropDraft(id); // a deliberate close discards the draft
  dropDraftImages(id);
  const h = await findHost(id);
  try {
    await control(h, F_SHUTDOWN);
  } catch {
    // Host unresponsive — take the whole process tree down.
    await new Promise((resolve) => {
      spawn("taskkill", ["/PID", String(h.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" })
        .on("exit", resolve)
        .on("error", resolve);
    });
  }
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline && (await hasSession(id))) await sleep(50);
}

// Same session identity, fresh process: kill then start in the same slot.
export async function respawnPane(id, dir, command, extraArgs = []) {
  if (await hasSession(id)) await killSession(id);
  await createSession(id, dir, command, extraArgs);
}

export async function renameSession(oldId, newId) {
  const h = await findHost(oldId);
  await control(h, F_RENAME, Buffer.from(newId, "utf8"));
}
