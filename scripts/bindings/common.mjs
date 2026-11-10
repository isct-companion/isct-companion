import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  existsSync,
  realpathSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
export const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
// Cargo's target directory. An external CARGO_TARGET_DIR wins, so several
// worktrees can share one and skip recompiling the dependency graph; the
// checkout's own target/ is the fallback. Every SDK path below derives from it.
export const target = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : join(root, "target");
export const output = join(target, "native-sdk");
export const generated = join(output, "generated");
export const iosDeploymentTarget = "17.0";
// This is the product compatibility floor, not the API level of the CI emulator.
// Lower it only after the complete AAR, JNA and OCR runtime pass on that API level.
export const androidMinSdk = 34;
export const androidCompileSdk = 34;
export function sdkFeatureArgs({ bindgen = false, ocr = false } = {}) {
  const features = [bindgen && "bindgen", ocr && "ocr"].filter(Boolean);
  return features.length > 0 ? ["--features", features.join(",")] : [];
}
export function run(cmd, args, options = {}) {
  console.log(`> ${cmd} ${args.join(" ")}`);
  const child = spawnSync(cmd, args, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: target },
    ...options,
  });
  if (child.error) throw child.error;
  if (child.status !== 0)
    throw new Error(`${cmd} failed (${child.status ?? child.signal})`);
}
export function capture(cmd, args) {
  return execFileSync(cmd, args, { cwd: root, encoding: "utf8" }).trim();
}
const windows = process.platform === "win32";
// kotlinc is a batch file on Windows, which Node starts only through cmd.exe,
// and cmd.exe re-splits the line at spaces in paths. The arguments go in a
// kotlinc @argfile instead, with forward slashes so none need escaping.
export function kotlinc(args) {
  if (!windows) return run("kotlinc", args);
  mkdirSync(output, { recursive: true });
  const file = join(output, "kotlinc.args");
  writeFileSync(
    file,
    args.map((arg) => `"${arg.replaceAll("\\", "/")}"`).join("\n"),
  );
  run(`kotlinc.bat "@${file}"`, [], { shell: true });
}
function kotlinRoot() {
  const kotlinc = capture(windows ? "where" : "which", ["kotlinc"]).split(
    /\r?\n/,
  )[0];
  return dirname(dirname(realpathSync(kotlinc)));
}
// The host compiler's "major.minor", from the distribution's build.txt
// ("2.2.21-release-469").
export function kotlincVersion() {
  const home = kotlinRoot();
  const build = ["build.txt", "libexec/build.txt"]
    .map((file) => join(home, file))
    .find(existsSync);
  const version =
    build && readFileSync(build, "utf8").match(/^(\d+\.\d+)/)?.[1];
  if (!version)
    throw new Error("Missing build.txt in the Kotlin compiler distribution");
  return version;
}
export function kotlinLibrary(name) {
  const home = kotlinRoot();
  const library = ["lib", "libexec/lib"]
    .map((directory) => join(home, directory, name))
    .find(existsSync);
  if (!library)
    throw new Error(`Missing ${name} in the Kotlin compiler distribution`);
  return library;
}
const sha256 = (file) =>
  createHash("sha256").update(readFileSync(file)).digest("hex");
// Pinned build dependency, fetched once into the SDK output and checked by
// hash. Mirrors drop connections now and then (ECONNRESET on CI), so a failed
// request is retried before the build gives up.
async function pinned(name, url, hash) {
  mkdirSync(output, { recursive: true });
  const file = join(output, name);
  for (let attempt = 1; !existsSync(file) || sha256(file) !== hash; attempt++) {
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      writeFileSync(file, Buffer.from(await response.arrayBuffer()));
      if (sha256(file) !== hash) throw new Error("checksum mismatch");
    } catch (error) {
      if (attempt === 4)
        throw new Error(`${name} download failed: ${error.message}`);
      console.warn(`${name} download failed (${error.message}); retrying`);
      await new Promise((done) => setTimeout(done, attempt * 5000));
    }
  }
  return file;
}
export async function javaDependencies() {
  const jar = await pinned(
    "jna-5.17.0.jar",
    "https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.jar",
    "b3a9408e7c51e08ef0e3bfcc08f443f6ec0f6191ba8cd7c18d53d2b22e5bdbc0",
  );
  const coroutines =
    process.env.KOTLIN_COROUTINES_JAR ??
    kotlinLibrary("kotlinx-coroutines-core-jvm.jar");
  return { jna: jar, coroutines };
}
export function generate() {
  mkdirSync(generated, { recursive: true });
  const library = join(
    target,
    "debug",
    { darwin: "libapp_ffi.dylib", win32: "app_ffi.dll" }[process.platform] ??
      "libapp_ffi.so",
  );
  run(join(target, "debug/isct-bindgen"), [
    "generate",
    library,
    "--language",
    "swift",
    "--language",
    "kotlin",
    "--no-format",
    "--out-dir",
    generated,
  ]);
}
export const androidJna = () =>
  pinned(
    "jna-5.17.0.aar",
    "https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.aar",
    "4dbeffffa665d97ad5aa7eee297531d3c841a86716ab7f774fd6956422b3cf38",
  );

export const androidAnnotations = () =>
  pinned(
    "annotation-jvm-1.9.1.jar",
    "https://dl.google.com/dl/android/maven2/androidx/annotation/annotation-jvm/1.9.1/annotation-jvm-1.9.1.jar",
    "1e343917ebf27ba96fe4dc52b1cad7fd32b738fbc6355bb6cd5b3b305d7212d0",
  );
