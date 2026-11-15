// `npm run tauri` entry point. Passes everything through to the Tauri CLI, except that `tauri dev` first picks a free
// dev-server port, so several checkouts or worktrees (or a browser-mode `npm run dev` on 1430) can run side by side.
//
//   npm run tauri dev                  first free port from 1430 up
//   KITCHAT_DEV_PORT=1440 npm run tauri dev   that port, or fail if it's taken
//
// The port goes to Tauri as an extra --config (devUrl + beforeDevCommand), merged over tauri.conf.json, and to Vite as
// `--port N --strictPort`. Release CI runs `npm run tauri build`, which this leaves untouched.

import { spawn } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_PORT = 1430;
const SEARCH_SPAN = 80;

/** Resolves true if nothing is listening on `port` at `host`. A host this machine doesn't have (no IPv6) counts as free. */
function canListen(port, host) {
  return new Promise((done) => {
    const server = createServer();
    server.once("error", (err) => done(err.code === "EADDRNOTAVAIL" || err.code === "EAFNOSUPPORT"));
    server.listen({ port, host, exclusive: true }, () => server.close(() => done(true)));
  });
}

/** Vite listens on `localhost`, which can resolve to either loopback address, so the port must be free on both. */
export async function isPortFree(port) {
  return (await canListen(port, "127.0.0.1")) && (await canListen(port, "::1"));
}

export async function findFreePort(start = DEFAULT_PORT, span = SEARCH_SPAN, isFree = isPortFree) {
  for (let port = start; port < start + span; port++) {
    if (await isFree(port)) return port;
  }
  throw new Error(`No free dev-server port in ${start}-${start + span - 1}.`);
}

export function devConfig(port) {
  return {
    build: {
      devUrl: `http://localhost:${port}`,
      beforeDevCommand: `npm run dev -- --port ${port} --strictPort`,
    },
  };
}

export async function choosePort(env, isFree = isPortFree) {
  if (env.KITCHAT_DEV_PORT) {
    const port = Number(env.KITCHAT_DEV_PORT);
    if (!Number.isInteger(port) || port < 1 || port > 65535)
      throw new Error(`KITCHAT_DEV_PORT=${env.KITCHAT_DEV_PORT} isn't a port.`);
    if (!(await isFree(port))) throw new Error(`KITCHAT_DEV_PORT=${port} is already in use.`);
    return port;
  }
  return findFreePort(DEFAULT_PORT, SEARCH_SPAN, isFree);
}

async function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const cli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  const args = process.argv.slice(2);
  let configFile = null;

  if (args[0] === "dev") {
    const port = await choosePort(process.env);
    if (port !== DEFAULT_PORT) console.log(`Port ${DEFAULT_PORT} is in use; the dev server will run on ${port}.`);
    // A file rather than inline JSON, which Windows shells mangle.
    configFile = join(tmpdir(), `kitchat-dev-${process.pid}.json`);
    writeFileSync(configFile, JSON.stringify(devConfig(port)));
    args.splice(1, 0, "--config", configFile);
  }

  const child = spawn(process.execPath, [cli, ...args], { cwd: root, stdio: "inherit" });
  // Ctrl-C reaches the CLI directly (same console); wait for it to shut Vite and the app down rather than dying first.
  process.on("SIGINT", () => {});
  child.on("exit", (code, signal) => {
    if (configFile) rmSync(configFile, { force: true });
    process.exit(code ?? (signal ? 1 : 0));
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((err) => {
    console.error(err.message);
    process.exit(1);
  });
}
