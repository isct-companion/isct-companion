import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const temporary = mkdtempSync(path.join(tmpdir(), "isct-icons-"));
const artwork = readFileSync(
  path.join(root, "backend/icons/source.png"),
).toString("base64");
const cli = path.join(root, "node_modules/@tauri-apps/cli/tauri.js");

// SVG supplies the platform mask; Tauri's existing renderer handles rasterization
// and every native icon format without changing the original artwork.
function generate(name, inset, pngSize) {
  const size = 1024 - inset * 2;
  const svg = path.join(temporary, `${name}.svg`);
  writeFileSync(
    svg,
    `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
    <defs><clipPath id="corners"><rect x="${inset}" y="${inset}" width="${size}" height="${size}" rx="${size * 0.22}"/></clipPath></defs>
    <image x="${inset}" y="${inset}" width="${size}" height="${size}" href="data:image/png;base64,${artwork}" clip-path="url(#corners)"/>
  </svg>`,
  );
  const output = path.join(temporary, name);
  const result = spawnSync(
    process.execPath,
    [
      cli,
      "icon",
      svg,
      "--output",
      output,
      ...(pngSize ? ["--png", String(pngSize)] : []),
    ],
    { cwd: temporary, stdio: "inherit" },
  );
  if (result.status !== 0) throw new Error(`Icon generation failed: ${name}`);
  return output;
}

try {
  const desktop = generate("desktop", 0);
  for (const name of readdirSync(desktop)) {
    if (name.endsWith(".png") || name.endsWith(".ico")) {
      copyFileSync(
        path.join(desktop, name),
        path.join(root, "backend/icons", name),
      );
    }
  }
  const mac = generate("mac", 64);
  copyFileSync(
    path.join(mac, "icon.icns"),
    path.join(root, "backend/icons/icon.icns"),
  );
  const web = generate("web", 0, 1024);
  copyFileSync(
    path.join(web, "1024x1024.png"),
    path.join(root, "public/icon.png"),
  );
  // Mobile icons intentionally keep full-bleed artwork: iOS and Android apply
  // their own platform masks. Do not copy desktop transparency into them.
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
