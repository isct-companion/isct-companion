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
export const target = join(root, "target");
export const output = join(target, "native-sdk");
export const generated = join(output, "generated");
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
export async function javaDependencies() {
  mkdirSync(output, { recursive: true });
  const jar = join(output, "jna-5.17.0.jar");
  const hash =
    "b3a9408e7c51e08ef0e3bfcc08f443f6ec0f6191ba8cd7c18d53d2b22e5bdbc0";
  if (!existsSync(jar)) {
    const response = await fetch(
      "https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.jar",
    );
    if (!response.ok)
      throw new Error(`JNA download failed: ${response.status}`);
    writeFileSync(jar, Buffer.from(await response.arrayBuffer()));
  }
  if (createHash("sha256").update(readFileSync(jar)).digest("hex") !== hash)
    throw new Error("JNA checksum mismatch");
  const kotlinRoot = dirname(
    dirname(realpathSync(capture("which", ["kotlinc"]))),
  );
  const coroutines =
    process.env.KOTLIN_COROUTINES_JAR ??
    ["lib", "libexec/lib"]
      .map((p) => join(kotlinRoot, p, "kotlinx-coroutines-core-jvm.jar"))
      .find(existsSync);
  if (!coroutines)
    throw new Error(
      "Set KOTLIN_COROUTINES_JAR to the compiler distribution coroutines jar",
    );
  return { jna: jar, coroutines };
}
export function generate() {
  mkdirSync(generated, { recursive: true });
  const library = join(
    target,
    "debug",
    process.platform === "darwin" ? "libisct_ffi.dylib" : "libisct_ffi.so",
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
export async function androidJna() {
  const file = join(output, "jna-5.17.0.aar");
  const hash =
    "4dbeffffa665d97ad5aa7eee297531d3c841a86716ab7f774fd6956422b3cf38";
  mkdirSync(output, { recursive: true });
  if (!existsSync(file)) {
    const response = await fetch(
      "https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.aar",
    );
    if (!response.ok)
      throw new Error(`JNA AAR download failed: ${response.status}`);
    writeFileSync(file, Buffer.from(await response.arrayBuffer()));
  }
  if (createHash("sha256").update(readFileSync(file)).digest("hex") !== hash)
    throw new Error("JNA AAR checksum mismatch");
  return file;
}
