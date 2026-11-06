// Campus VPN payload for the Windows app: the helper (also its own installer
// and route hook), the JScript shim openconnect runs the hook through, and
// openconnect with the libraries it loads, written to
// apps/desktop/src-tauri/resources/vpn for `tauri.mjs`. The helper's
// installer copies it under Program Files on the person's first connect.
//
// openconnect comes from the official MinGW64/GnuTLS Windows build
// (GitLab CI job of the release tag), unpacked or installed:
// `--openconnect <dir>` names the directory with openconnect.exe, default
// %ProgramFiles%\OpenConnect. Unpack the installer with
// `7z x openconnect-installer-MinGW64-GnuTLS-v9.21.exe -o<dir>`. Only the
// DLLs openconnect.exe loads are taken, found from the PE import tables.
// Wintun comes from wintun.net, pinned by hash.
//
// The official build carries no license texts, so each DLL is matched to its
// project and SPDX license below and the texts are fetched from the SPDX
// license list; a DLL the table does not know stops the bundle.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";

const WINTUN = {
  version: "0.14.1",
  url: "https://www.wintun.net/builds/wintun-0.14.1.zip",
  sha256: "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51",
};
const SPDX_TEXTS = "https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text";

/** DLL file name → the project it comes from. */
const COMPONENTS = [
  [/^(openconnect\.exe|libopenconnect-\d+\.dll)$/i, "openconnect", "LGPL-2.1-only", "https://gitlab.com/openconnect/openconnect"],
  [/^libgnutls-\d+\.dll$/i, "gnutls", "LGPL-2.1-or-later", "https://gnutls.org/"],
  [/^lib(nettle|hogweed)-\d+\.dll$/i, "nettle", "LGPL-3.0-or-later OR GPL-2.0-or-later", "https://www.lysator.liu.se/~nisse/nettle/"],
  [/^libgmp-\d+\.dll$/i, "gmp", "LGPL-3.0-or-later OR GPL-2.0-or-later", "https://gmplib.org/"],
  [/^libtasn1-\d+\.dll$/i, "libtasn1", "LGPL-2.1-or-later", "https://www.gnu.org/software/libtasn1/"],
  [/^libp11-kit-\d+\.dll$/i, "p11-kit", "BSD-3-Clause", "https://p11-glue.github.io/p11-glue/p11-kit.html"],
  [/^libidn2-\d+\.dll$/i, "libidn2", "LGPL-3.0-or-later OR GPL-2.0-or-later", "https://www.gnu.org/software/libidn/"],
  [/^libunistring-\d+\.dll$/i, "libunistring", "LGPL-3.0-or-later OR GPL-2.0-or-later", "https://www.gnu.org/software/libunistring/"],
  [/^libffi-\d+\.dll$/i, "libffi", "MIT", "https://sourceware.org/libffi/"],
  [/^libxml2-\d+\.dll$/i, "libxml2", "MIT", "https://gitlab.gnome.org/GNOME/libxml2"],
  [/^zlib1\.dll$/i, "zlib", "Zlib", "https://zlib.net/"],
  [/^liblz4\.dll$/i, "lz4", "BSD-2-Clause", "https://github.com/lz4/lz4"],
  [/^libstoken-\d+\.dll$/i, "stoken", "LGPL-2.1-or-later", "https://github.com/stoken-dev/stoken"],
  [/^libintl-\d+\.dll$/i, "gettext-runtime", "LGPL-2.1-or-later", "https://www.gnu.org/software/gettext/"],
  // Fedora's MinGW iconv.dll is win-iconv, placed in the public domain.
  [/^iconv\.dll$/i, "win-iconv", "LicenseRef-Public-Domain", "https://github.com/win-iconv/win-iconv"],
  [/^libiconv-\d+\.dll$/i, "libiconv", "LGPL-2.1-or-later", "https://www.gnu.org/software/libiconv/"],
  [/^libgcc_s_seh-\d+\.dll$/i, "libgcc", "GPL-3.0-or-later WITH GCC-exception-3.1", "https://gcc.gnu.org/"],
  [/^libwinpthread-\d+\.dll$/i, "winpthreads", "MIT AND BSD-3-Clause", "https://www.mingw-w64.org/"],
  [/^wintun\.dll$/i, "wintun", "LicenseRef-Wintun", "https://www.wintun.net/"],
];

const PUBLIC_DOMAIN = "This software is placed in the public domain by its authors.";

/** The DLL names a PE file imports, including delay-loaded ones. */
export function peImports(file) {
  const b = readFileSync(file);
  const pe = b.readUInt32LE(0x3c);
  if (b.toString("latin1", pe, pe + 4) !== "PE\0\0") throw new Error(`${file} is not a PE file`);
  const sections = b.readUInt16LE(pe + 6);
  const optional = pe + 24;
  const optionalSize = b.readUInt16LE(pe + 20);
  const directories = optional + (b.readUInt16LE(optional) === 0x20b ? 112 : 96);
  const table = optional + optionalSize;
  const offset = (rva) => {
    for (let i = 0; i < sections; i++) {
      const at = table + 40 * i;
      const [size, va, raw] = [b.readUInt32LE(at + 8), b.readUInt32LE(at + 12), b.readUInt32LE(at + 20)];
      const span = Math.max(size, b.readUInt32LE(at + 16));
      if (rva >= va && rva < va + span) return raw + rva - va;
    }
    throw new Error(`${file}: RVA ${rva} is outside every section`);
  };
  const name = (rva) => {
    const at = offset(rva);
    return b.toString("latin1", at, b.indexOf(0, at));
  };
  const names = [];
  // Import directory: 20-byte descriptors, name RVA at +12.
  const imports = b.readUInt32LE(directories + 8);
  for (let at = imports && offset(imports); imports; at += 20) {
    const rva = b.readUInt32LE(at + 12);
    if (!rva) break;
    names.push(name(rva));
  }
  // Delay-load directory: 32-byte descriptors, name RVA at +4.
  const delayed = b.readUInt32LE(directories + 13 * 8);
  for (let at = delayed && offset(delayed); delayed; at += 32) {
    const rva = b.readUInt32LE(at + 4);
    if (!rva) break;
    names.push(name(rva));
  }
  return names;
}

function componentOf(file) {
  const entry = COMPONENTS.find(([pattern]) => pattern.test(file));
  if (!entry) throw new Error(`${file} has no entry in bundle-vpn-windows.mjs; add its project and license`);
  const [, name, license, url] = entry;
  return { name, license, url };
}

/**
 * The product version in a PE version resource, when it has one; the
 * openconnect library has none but carries its `openconnect_version_str`.
 */
function peVersion(file) {
  const bytes = readFileSync(file);
  if (/^libopenconnect-\d+\.dll$/i.test(path.basename(file))) {
    return /\0v(\d+\.\d+(?:\.\d+)?)\0/.exec(bytes.toString("latin1"))?.[1] ?? "";
  }
  const match = /ProductVersion\0+([0-9][0-9A-Za-z.\-+]*)/.exec(bytes.toString("utf16le"));
  return match?.[1] ?? "";
}

const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");

// curl.exe ships with Windows and follows the system's proxy settings,
// which Node's fetch does not.
const download = (url, file) => execFileSync("curl", ["-fsSL", "-o", file, url]);

/** The license ids an SPDX expression names, exceptions included. */
const licenseIds = (expression) =>
  expression
    .replace(/[()]/g, " ")
    .split(/\s+(?:AND|OR|WITH)\s+|\s+/)
    .filter(Boolean);

export async function bundleWindows({ openconnect: source, out }) {
  const binary = path.join(source, "openconnect.exe");
  if (!existsSync(binary)) {
    throw new Error(`${binary} not found; pass --openconnect <dir with openconnect.exe>`);
  }
  mkdirSync(out, { recursive: true });
  const local = new Map(readdirSync(source).map((file) => [file.toLowerCase(), file]));

  // Every DLL the binary loads that ships beside it, transitively; the rest
  // are Windows' own.
  const taken = new Set(["openconnect.exe"]);
  const queue = [binary];
  while (queue.length) {
    for (const dll of peImports(queue.shift())) {
      const file = local.get(dll.toLowerCase());
      if (!file || taken.has(file)) continue;
      taken.add(file);
      queue.push(path.join(source, file));
    }
  }
  for (const file of taken) copyFileSync(path.join(source, file), path.join(out, file));

  // openconnect loads wintun.dll at run time, so no import names it.
  const scratch = mkdtempSync(path.join(os.tmpdir(), "isct-wintun-"));
  try {
    const zip = path.join(scratch, "wintun.zip");
    download(WINTUN.url, zip);
    if (sha256(zip) !== WINTUN.sha256) throw new Error(`${WINTUN.url} does not match its pinned hash`);
    // Windows' own tar is bsdtar, which reads zip; a GNU tar earlier on PATH
    // (Git Bash) would take `C:` for a remote host.
    const tar = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
    execFileSync(tar, ["-xf", zip, "-C", scratch]);
    copyFileSync(path.join(scratch, "wintun/bin/amd64/wintun.dll"), path.join(out, "wintun.dll"));
    taken.add("wintun.dll");
    mkdirSync(path.join(out, "licenses"), { recursive: true });
    copyFileSync(path.join(scratch, "wintun/LICENSE.txt"), path.join(out, "licenses/LicenseRef-Wintun.txt"));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }

  const components = new Map();
  for (const file of [...taken].sort()) {
    const component = componentOf(file);
    const known = components.get(component.name);
    const version =
      component.name === "wintun" ? WINTUN.version : peVersion(path.join(out, file));
    components.set(component.name, { ...component, version: known?.version || version });
  }
  const licenses = path.join(out, "licenses");
  for (const id of new Set([...components.values()].flatMap((item) => licenseIds(item.license)))) {
    const file = path.join(licenses, `${id}.txt`);
    if (existsSync(file)) continue;
    if (id === "LicenseRef-Public-Domain") writeFileSync(file, PUBLIC_DOMAIN + "\n");
    else download(`${SPDX_TEXTS}/${id}.txt`, file);
  }
  const list = [...components.values()]
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((item) => ({
      name: item.name,
      version: item.version,
      license: item.license,
      url: item.url,
      files: licenseIds(item.license).map((id) => `licenses/${id}.txt`),
    }));
  writeFileSync(path.join(out, "components.json"), JSON.stringify(list, null, 2) + "\n");
  return { libraries: taken.size - 1, components: list };
}
