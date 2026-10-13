// Third-party notices: the license texts of everything shipped in the app.
// MIT, BSD and Apache components require their notices to travel with the
// binary, so the build collects them instead of keeping a list by hand. The
// file lands in dist/ and is copied into the public release repository.
//
// - Frontend: npm packages that actually end up in the bundle.
// - Native: Rust crates linked into the backend (normal dependencies only),
//   collected only for Tauri builds (TAURI_ENV_PLATFORM is set).
// - Models: the bundled OCR model and runtime notices.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const LICENSE_FILE = /^(licen[cs]e|copying|notice)/i;
export const NOTICES_FILE = "THIRD-PARTY-NOTICES.txt";

function licenseTexts(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => LICENSE_FILE.test(name))
    .sort()
    .map((name) => readFileSync(path.join(dir, name), "utf8").trim())
    .filter(Boolean);
}

/** The package root that owns a bundled module path inside node_modules. */
export function packageRoot(moduleId) {
  const file = moduleId.replace(/^\0/, "").split("?")[0];
  const marker = `${path.sep}node_modules${path.sep}`;
  const at = file.lastIndexOf(marker);
  if (at < 0) return undefined;
  const rest = file.slice(at + marker.length).split(path.sep);
  const depth = rest[0].startsWith("@") ? 2 : 1;
  return file.slice(0, at + marker.length) + rest.slice(0, depth).join(path.sep);
}

function npmComponents(moduleIds) {
  const roots = new Set(moduleIds.map(packageRoot).filter(Boolean));
  return [...roots].flatMap((dir) => {
    const manifest = path.join(dir, "package.json");
    if (!existsSync(manifest)) return [];
    const pkg = JSON.parse(readFileSync(manifest, "utf8"));
    const repository =
      typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
    return [
      {
        group: "app",
        name: pkg.name,
        version: pkg.version,
        license:
          typeof pkg.license === "string"
            ? pkg.license
            : pkg.license?.type ?? "UNKNOWN",
        url: pkg.homepage ?? repository,
        texts: licenseTexts(dir),
      },
    ];
  });
}

function hostTriple() {
  const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  return /^host: (.+)$/m.exec(info)?.[1];
}

function crateComponents(target) {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      [
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--manifest-path",
        path.join(root, "backend/Cargo.toml"),
        "--filter-platform",
        target,
      ],
      { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    ),
  );
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const own = metadata.resolve.root;
  // Walk normal dependencies only; build and dev dependencies do not ship.
  const shipped = new Set();
  const stack = [own];
  while (stack.length) {
    const id = stack.pop();
    if (shipped.has(id)) continue;
    shipped.add(id);
    for (const dep of nodes.get(id)?.deps ?? [])
      if (dep.dep_kinds.some((kind) => kind.kind === null)) stack.push(dep.pkg);
  }
  return metadata.packages
    .filter((pkg) => shipped.has(pkg.id) && pkg.id !== own)
    .map((pkg) => ({
      group: "native",
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? "UNKNOWN",
      url: pkg.repository ?? pkg.homepage ?? undefined,
      authors: pkg.authors?.length ? pkg.authors : undefined,
      texts: licenseTexts(path.dirname(pkg.manifest_path)),
    }));
}

function modelComponents() {
  const dir = path.join(root, "crates/isct-ocr/models/paddle");
  const read = (name) => {
    const file = path.join(dir, name);
    return existsSync(file) ? [readFileSync(file, "utf8").trim()] : [];
  };
  return [
    {
      group: "models",
      name: "PaddleOCR PP-OCRv5 model",
      license: "Apache-2.0",
      url: "https://github.com/PaddlePaddle/PaddleOCR",
      texts: read("PADDLE-LICENSE"),
    },
    {
      group: "models",
      name: "ONNX Runtime",
      license: "MIT",
      url: "https://github.com/microsoft/onnxruntime",
      texts: [
        ...read("ONNX-RUNTIME-LICENSE"),
        ...read("ONNX-THIRD-PARTY-NOTICES.txt"),
      ],
    },
  ];
}

/**
 * Builds the notices document. Identical license texts are stored once and
 * referenced by index, which keeps hundreds of Apache/MIT copies small.
 */
export function buildNotices({ moduleIds, native }) {
  const components = [
    ...npmComponents(moduleIds),
    ...(native ? crateComponents(native) : []),
    ...modelComponents(),
  ].sort(
    (a, b) =>
      a.group.localeCompare(b.group) ||
      a.name.localeCompare(b.name) ||
      String(a.version ?? "").localeCompare(String(b.version ?? "")),
  );
  // A few packages ship no license file; name the license and point to it.
  for (const component of components)
    if (!component.texts.length)
      component.texts = [
        [
          `Licensed under ${component.license}.`,
          component.authors?.length
            ? `Copyright (c) ${component.authors.join(", ")}.`
            : undefined,
          ...component.license
            .split(/\s+OR\s+|\s+AND\s+|\/|[()]/)
            .map((id) => id.trim())
            .filter(Boolean)
            .map((id) => `https://spdx.org/licenses/${id}.html`),
        ]
          .filter(Boolean)
          .join("\n"),
      ];
  const texts = [];
  const index = new Map();
  const ref = (text) => {
    if (!index.has(text)) {
      index.set(text, texts.length);
      texts.push(text);
    }
    return index.get(text);
  };
  return {
    components: components.map(({ texts: own, ...rest }) => ({
      ...rest,
      texts: own.map(ref),
    })),
    texts,
  };
}

const GROUP_TITLES = {
  app: "Interface (npm packages)",
  native: "Native app (Rust crates)",
  models: "OCR model and runtime",
};

/** Plain-text notices: one section per component, shared texts inlined. */
export function renderNotices({ components, texts }) {
  const rule = "=".repeat(78);
  const lines = [
    "IsctCompanion includes the following third-party software.",
    "",
  ];
  for (const [group, title] of Object.entries(GROUP_TITLES)) {
    const items = components.filter((item) => item.group === group);
    if (!items.length) continue;
    lines.push(rule, title, rule, "");
    for (const item of items) {
      lines.push(
        `${item.name}${item.version ? ` ${item.version}` : ""} (${item.license})`,
      );
      if (item.url) lines.push(item.url);
      lines.push("", ...item.texts.map((index) => texts[index] + "\n"), "-".repeat(78), "");
    }
  }
  return lines.join("\n");
}

/** Vite plugin: emits THIRD-PARTY-NOTICES.txt next to the built app. */
export function thirdPartyNotices() {
  return {
    name: "isct-third-party-notices",
    apply: "build",
    generateBundle() {
      const platform = process.env.TAURI_ENV_PLATFORM;
      let native;
      if (platform) {
        // A native build must ship the crate notices; fail loudly without them.
        native = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
      }
      const notices = buildNotices({ moduleIds: [...this.getModuleIds()], native });
      this.emitFile({
        type: "asset",
        fileName: NOTICES_FILE,
        source: renderNotices(notices),
      });
    },
  };
}
