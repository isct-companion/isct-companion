// Interaction trace sink for `vite serve` (and so `yarn desktop:dev`).
// The dev webview posts one timeline of clicks, route changes, Tauri invokes,
// backend events and errors (apps/desktop/src/app/interaction-trace.ts); this
// plugin appends it to tmp/interaction-trace/<page load>.jsonl and, when an
// error arrives, prints the steps that led to it in the dev terminal. rrweb and
// similar recorders replay the DOM but not the native IPC, which is the half
// that matters here, so the recorder is a small custom one.
import { appendFileSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = fileURLToPath(
  new URL("../../tmp/interaction-trace/", import.meta.url),
);
const tracedCore = fileURLToPath(
  new URL("../../apps/desktop/src/app/traced-tauri-core.ts", import.meta.url),
);
const KEEP_FILES = 20;
const LEAD_UP = 25;
const SESSION = /^[\w.-]{1,80}$/;

function prune() {
  const files = readdirSync(dir)
    .filter((name) => name.endsWith(".jsonl"))
    .sort();
  for (const name of files.slice(0, -KEEP_FILES))
    rmSync(path.join(dir, name), { force: true });
}

const line = (event) => {
  const { t, kind, ...rest } = event;
  const at = `+${(t / 1000).toFixed(3)}s`.padStart(10);
  return `${at} ${kind.padEnd(9)} ${JSON.stringify(rest).slice(0, 220)}`;
};

export function interactionTrace() {
  const recent = new Map();
  return {
    name: "isct-interaction-trace",
    apply: "serve",
    enforce: "pre",
    // App imports of the Tauri core module go through the traced wrapper;
    // the wrapper itself (and the Tauri package) still get the real one.
    resolveId(source, importer) {
      if (
        source === "@tauri-apps/api/core" &&
        importer &&
        importer !== tracedCore &&
        !importer.includes("/node_modules/")
      )
        return tracedCore;
    },
    configureServer(server) {
      mkdirSync(dir, { recursive: true });
      server.middlewares.use("/__interaction-trace", (req, res) => {
        if (req.method !== "POST") {
          res.statusCode = 405;
          return res.end();
        }
        let body = "";
        req.setEncoding("utf8");
        req.on("data", (chunk) => (body += chunk));
        req.on("end", () => {
          res.statusCode = 204;
          res.end();
          let batch;
          try {
            batch = JSON.parse(body);
          } catch {
            return;
          }
          if (
            !SESSION.test(batch?.session ?? "") ||
            !Array.isArray(batch.events)
          )
            return;
          const file = path.join(dir, `${batch.session}.jsonl`);
          if (!recent.has(batch.session)) {
            recent.set(batch.session, []);
            prune();
            server.config.logger.info(
              `[trace] recording to ${path.relative(process.cwd(), file)}`,
            );
          }
          appendFileSync(
            file,
            batch.events.map((event) => JSON.stringify(event) + "\n").join(""),
          );
          const tail = recent.get(batch.session);
          for (const event of batch.events) {
            if (event.kind === "error") {
              server.config.logger.error(
                `[trace] ${event.source}: ${event.message}\n` +
                  [...tail.slice(-LEAD_UP), event].map(line).join("\n") +
                  `\n[trace] full timeline: ${file}`,
              );
            }
            tail.push(event);
          }
          tail.splice(0, Math.max(0, tail.length - LEAD_UP));
        });
      });
    },
  };
}
