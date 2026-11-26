// Campus VPN payload for the macOS app: the root helper (with the route hook
// compiled in) and a self-contained openconnect, written to
// apps/desktop/src-tauri/resources/vpn where `tauri.mjs` picks it up. launchd
// runs the helper from there as an SMAppService daemon. On Windows the payload is built by
// `bundle-vpn-windows.mjs` instead; the command line is the same.
//
// openconnect comes from a Homebrew keg (`--openconnect <keg>`, default
// `brew --prefix openconnect`). Its libraries are found the way Homebrew
// would, copied next to it, and every reference is rewritten to
// @loader_path, so nothing under /opt/homebrew is needed at run time. Each
// formula's license files and SPDX license go along for the notices
// (`third-party-notices.mjs`).
//
//   node scripts/desktop/bundle-vpn.mjs [--openconnect <keg>] [--cellar <dir>]
//
// `--cellar` adds a directory of unpacked bottles (`<formula>/<version>/…`)
// searched before Homebrew, for kegs that are not installed.
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { bundleWindows } from "./bundle-vpn-windows.mjs";
import { hasCampusVpn } from "./desktop-target.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
export const VPN_RESOURCES = path.join(
  root,
  "apps/desktop/src-tauri/resources/vpn",
);

const run = (command, args, options = {}) =>
  execFileSync(command, args, { encoding: "utf8", ...options }).trim();

function option(name) {
  const at = process.argv.indexOf(name);
  return at < 0 ? undefined : process.argv[at + 1];
}

const brewPrefix = (() => {
  try {
    return run("brew", ["--prefix"]);
  } catch {
    return "/opt/homebrew";
  }
})();
const cellars = [option("--cellar"), path.join(brewPrefix, "Cellar")].filter(
  Boolean,
);

/** A library reference as Homebrew wrote it, resolved to a file here. */
function resolve(reference, from) {
  if (reference.startsWith("/usr/lib/") || reference.startsWith("/System/"))
    return null;
  const candidates = [];
  if (reference.startsWith("@@HOMEBREW_CELLAR@@/")) {
    const rest = reference.slice("@@HOMEBREW_CELLAR@@/".length);
    for (const cellar of cellars) candidates.push(path.join(cellar, rest));
  } else if (reference.startsWith("@@HOMEBREW_PREFIX@@/opt/")) {
    const rest = reference.slice("@@HOMEBREW_PREFIX@@/opt/".length);
    const [name, ...file] = rest.split("/");
    for (const cellar of cellars) {
      const dir = path.join(cellar, name);
      if (!existsSync(dir)) continue;
      for (const version of readdirSync(dir).sort().reverse())
        candidates.push(path.join(dir, version, ...file));
    }
    candidates.push(path.join(brewPrefix, "opt", rest));
  } else if (
    reference.startsWith("@loader_path/") ||
    reference.startsWith("@rpath/")
  ) {
    candidates.push(path.join(path.dirname(from), path.basename(reference)));
  } else {
    candidates.push(reference);
  }
  const found = candidates.find((file) => existsSync(file));
  if (!found) throw new Error(`cannot find ${reference} (needed by ${from})`);
  return found;
}

/** The non-system libraries `file` links, as `otool -L` lists them. */
function linked(file) {
  return run("otool", ["-L", file])
    .split("\n")
    .slice(1)
    .map((line) => line.trim().split(" (")[0])
    .filter(Boolean);
}

/** The keg (`<cellar>/<formula>/<version>`) a resolved file belongs to. */
function kegOf(file) {
  const real = run("realpath", [file]);
  for (const cellar of [...cellars, path.join(brewPrefix, "Cellar")]) {
    const realCellar = existsSync(cellar) ? run("realpath", [cellar]) : cellar;
    if (real.startsWith(realCellar + path.sep)) {
      const [name, version] = real.slice(realCellar.length + 1).split(path.sep);
      return { name, version, dir: path.join(realCellar, name, version) };
    }
  }
  throw new Error(`${file} is not inside a Homebrew keg`);
}

function licenseOf(keg) {
  const sbom = path.join(keg.dir, "sbom.spdx.json");
  if (!existsSync(sbom)) return "NOASSERTION";
  const packages = JSON.parse(readFileSync(sbom, "utf8")).packages ?? [];
  const own = packages.find((item) => item.name === keg.name) ?? packages[0];
  // Homebrew records what it found in the source as the concluded license.
  return (
    [own?.licenseDeclared, own?.licenseConcluded].find(
      (license) => license && license !== "NOASSERTION",
    ) ?? "NOASSERTION"
  );
}

/**
 * Ad hoc, or with `APPLE_SIGNING_IDENTITY` as notarization requires of every
 * executable in the app: hardened runtime and a secure timestamp. Tauri signs
 * the app itself but nothing under Resources.
 */
function signMacho(file, { releaseIdentity, developmentIdentity } = {}) {
  execFileSync(
    "codesign",
    releaseIdentity
      ? [
          "--force",
          "--options",
          "runtime",
          "--timestamp",
          "--sign",
          releaseIdentity,
          file,
        ]
      : ["--force", "--sign", developmentIdentity ?? "-", file],
    { stdio: "ignore" },
  );
}

function bundleOpenconnect(keg, out, signing) {
  const bin = path.join(out, "bin");
  const lib = path.join(out, "lib");
  mkdirSync(bin, { recursive: true });
  mkdirSync(lib, { recursive: true });
  const binary = path.join(keg, "bin/openconnect");
  const target = path.join(bin, "openconnect");
  copyFileSync(binary, target);
  chmodSync(target, 0o755);

  const kegs = new Map();
  const note = (file) => {
    const owner = kegOf(file);
    kegs.set(owner.name, owner);
  };
  note(binary);
  // Copy every library the binary needs, transitively, by file name.
  const copied = new Map();
  const queue = [[binary, target]];
  while (queue.length) {
    const [source, copy] = queue.shift();
    for (const reference of linked(source)) {
      const found = resolve(reference, source);
      if (!found || run("realpath", [found]) === run("realpath", [source]))
        continue;
      const name = path.basename(found);
      execFileSync("install_name_tool", [
        "-change",
        reference,
        copy === target
          ? `@loader_path/../lib/${name}`
          : `@loader_path/${name}`,
        copy,
      ]);
      if (copied.has(name)) continue;
      const libCopy = path.join(lib, name);
      copyFileSync(found, libCopy);
      chmodSync(libCopy, 0o755);
      execFileSync("install_name_tool", [
        "-id",
        `@loader_path/${name}`,
        libCopy,
      ]);
      copied.set(name, libCopy);
      note(found);
      queue.push([found, libCopy]);
    }
  }
  // install_name_tool invalidates signatures; arm64 needs at least ad hoc.
  for (const file of [target, ...copied.values()]) signMacho(file, signing);

  const left = [target, ...copied.values()].flatMap((file) =>
    linked(file).filter((reference) =>
      /HOMEBREW|\/opt\/homebrew|\/usr\/local\//.test(reference),
    ),
  );
  if (left.length)
    throw new Error(`still linked to Homebrew: ${left.join(", ")}`);

  const licenses = path.join(out, "licenses");
  const components = [];
  for (const owner of [...kegs.values()].sort((a, b) =>
    a.name.localeCompare(b.name),
  )) {
    const dir = path.join(licenses, owner.name);
    mkdirSync(dir, { recursive: true });
    const files = readdirSync(owner.dir).filter((name) =>
      /^(licen[cs]e|copying|notice)/i.test(name),
    );
    for (const name of files)
      copyFileSync(path.join(owner.dir, name), path.join(dir, name));
    components.push({
      name: owner.name,
      version: owner.version,
      license: licenseOf(owner),
      files: files.map((name) => `licenses/${owner.name}/${name}`),
    });
  }
  writeFileSync(
    path.join(out, "components.json"),
    JSON.stringify(components, null, 2) + "\n",
  );
  return { libraries: copied.size, components };
}

/**
 * The team of a Developer ID identity such as `Developer ID Application:
 * Name (TEAMID)`. The macOS helper refuses an openconnect another team signed.
 */
const signerTeam = (identity) => identity?.match(/\(([A-Z0-9]{10})\)\s*$/)?.[1];

/** The helper built for release, copied into the payload. */
function buildHelper(binary, releaseIdentity) {
  const team = signerTeam(releaseIdentity);
  if (releaseIdentity && !team)
    throw new Error("APPLE_SIGNING_IDENTITY names no team (…(TEAMID))");
  const env = { ...process.env };
  delete env.ISCT_VPN_SIGNER_TEAM;
  if (team) env.ISCT_VPN_SIGNER_TEAM = team;
  execFileSync(
    "cargo",
    ["build", "--release", "--locked", "-p", "vpn-helper"],
    {
      cwd: root,
      stdio: "inherit",
      env,
    },
  );
  const helper = path.join(VPN_RESOURCES, binary);
  const targetDir = path.resolve(root, env.CARGO_TARGET_DIR ?? "target");
  copyFileSync(path.join(targetDir, "release", binary), helper);
  return helper;
}

/**
 * Rebuilds the helper in an existing payload, so a desktop build never ships
 * one older than the app's protocol (`HELPER_REVISION`): the app would call
 * it outdated, and an update would install the same stale file again.
 * openconnect stays as `desktop:vpn` built it. Cargo makes this a no-op when
 * the helper's sources have not changed.
 */
function refreshHelper(signing) {
  if (process.platform === "win32") {
    buildHelper("isct-vpn-helper.exe");
    return;
  }
  const helper = buildHelper("isct-vpn-helper", signing.releaseIdentity);
  chmodSync(helper, 0o755);
  // launchd records the designated requirement at registration. The app and its
  // helper use the same certificate so local rebuilds keep a stable requirement.
  signMacho(helper, signing);
}

export function campusVpnBundle({ target, macOS, packagesApp, signing }) {
  if (!hasCampusVpn(target)) return undefined;
  const helper = { darwin: "isct-vpn-helper", win32: "isct-vpn-helper.exe" }[
    process.platform
  ];
  if (!helper || !existsSync(path.join(VPN_RESOURCES, helper)))
    return undefined;

  if (packagesApp) refreshHelper(signing);
  console.log("Campus VPN payload: included.");
  // Tauri replaces resources on config merge. Extend the platform's list or map.
  const desktop = path.join(root, "apps/desktop/src-tauri");
  const config =
    process.platform === "win32"
      ? "tauri.windows.conf.json"
      : "tauri.conf.json";
  const { resources } = JSON.parse(
    readFileSync(path.join(desktop, config), "utf8"),
  ).bundle;
  return {
    resources: Array.isArray(resources)
      ? [...resources, "resources/vpn/"]
      : { ...resources, "resources/vpn/": "resources/vpn/" },
    ...(process.platform === "darwin" && {
      macOS: {
        ...macOS,
        files: {
          ...macOS?.files,
          "Library/LaunchDaemons/app.isctcompanion.vpn.plist":
            "../../../crates/vpn-helper/assets/app.isctcompanion.vpn.plist",
        },
      },
    }),
  };
}

async function mainWindows() {
  const source =
    option("--openconnect") ??
    path.join(process.env.ProgramFiles ?? "C:\\Program Files", "OpenConnect");
  rmSync(VPN_RESOURCES, { recursive: true, force: true });
  mkdirSync(VPN_RESOURCES, { recursive: true });
  buildHelper("isct-vpn-helper.exe");
  copyFileSync(
    path.join(root, "crates/vpn-helper/assets/isct-vpn-script.js"),
    path.join(VPN_RESOURCES, "isct-vpn-script.js"),
  );
  const { libraries, components } = await bundleWindows({
    openconnect: source,
    out: path.join(VPN_RESOURCES, "openconnect"),
  });
  console.log(
    `Campus VPN payload: helper, route hook, openconnect with ${libraries} libraries from ${components.length} projects.`,
  );
}

function main() {
  const signing = {
    releaseIdentity: process.env.APPLE_SIGNING_IDENTITY?.trim(),
  };
  const keg =
    option("--openconnect") ??
    (() => {
      try {
        return run("brew", ["--prefix", "openconnect"]);
      } catch {
        throw new Error(
          "openconnect is not installed; pass --openconnect <keg>",
        );
      }
    })();
  rmSync(VPN_RESOURCES, { recursive: true, force: true });
  mkdirSync(VPN_RESOURCES, { recursive: true });

  const helper = buildHelper("isct-vpn-helper", signing.releaseIdentity);
  chmodSync(helper, 0o755);
  signMacho(helper, signing);

  const { libraries, components } = bundleOpenconnect(
    keg,
    path.join(VPN_RESOURCES, "openconnect"),
    signing,
  );
  const arch = run("lipo", [
    "-archs",
    path.join(VPN_RESOURCES, "openconnect/bin/openconnect"),
  ]);
  console.log(
    `Campus VPN payload: helper, openconnect (${arch}) with ${libraries} libraries from ${components.length} formulae.`,
  );
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    await (process.platform === "win32" ? mainWindows() : main());
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
