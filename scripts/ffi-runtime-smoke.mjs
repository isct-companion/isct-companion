import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Isolated fixture, not the application FFI API. All output shares root target/.
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixture = join(root, "scripts/fixtures/ffi-runtime");
const target = join(root, "target");
const output = join(target, "ffi-runtime-smoke");
const generated = join(output, "generated");
if (process.platform !== "darwin") {
  throw new Error(
    "This host smoke test requires macOS, Swift 6, Kotlin and a JDK.",
  );
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: target },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.status ?? result.signal})`);
  }
}

await mkdir(output, { recursive: true });
const jna = join(output, "jna-5.17.0.jar");
const jnaHash =
  "b3a9408e7c51e08ef0e3bfcc08f443f6ec0f6191ba8cd7c18d53d2b22e5bdbc0";
let jar = await readFile(jna).catch(() => undefined);
if (!jar) {
  const response = await fetch(
    "https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.17.0/jna-5.17.0.jar",
  );
  if (!response.ok) throw new Error(`JNA download failed: ${response.status}`);
  jar = Buffer.from(await response.arrayBuffer());
}
if (createHash("sha256").update(jar).digest("hex") !== jnaHash) {
  throw new Error("JNA checksum mismatch");
}
await writeFile(jna, jar);
const kotlinRoot = dirname(
  dirname(
    realpathSync(
      execFileSync("which", ["kotlinc"], { encoding: "utf8" }).trim(),
    ),
  ),
);
const coroutines =
  process.env.KOTLIN_COROUTINES_JAR ??
  ["lib", "libexec/lib"]
    .map((path) => join(kotlinRoot, path, "kotlinx-coroutines-core-jvm.jar"))
    .find(existsSync);
if (!coroutines || !existsSync(coroutines)) {
  throw new Error("Set KOTLIN_COROUTINES_JAR to the installed coroutines jar.");
}

run("cargo", [
  "build",
  "--locked",
  "--manifest-path",
  join(fixture, "Cargo.toml"),
]);
run(join(target, "debug/bindgen"), [
  "generate",
  join(target, "debug/libisct_ffi_runtime_spike.dylib"),
  "--language",
  "swift",
  "--language",
  "kotlin",
  "--no-format",
  "--out-dir",
  generated,
]);
run("swiftc", [
  "-parse-as-library",
  "-swift-version",
  "6",
  "-module-cache-path",
  join(output, "ModuleCache"),
  join(generated, "isct_ffi_runtime_spike.swift"),
  join(fixture, "Smoke.swift"),
  "-Xcc",
  `-fmodule-map-file=${join(generated, "isct_ffi_runtime_spikeFFI.modulemap")}`,
  "-I",
  generated,
  "-L",
  join(target, "debug"),
  "-lisct_ffi_runtime_spike",
  "-Xlinker",
  "-rpath",
  "-Xlinker",
  join(target, "debug"),
  "-o",
  join(output, "swift-smoke"),
]);
run(join(output, "swift-smoke"), []);
const classpath = `${jna}:${coroutines}`;
run("kotlinc", [
  join(generated, "uniffi/isct_ffi_runtime_spike/isct_ffi_runtime_spike.kt"),
  join(fixture, "Smoke.kt"),
  "-classpath",
  classpath,
  "-d",
  join(output, "kotlin-smoke.jar"),
]);
run("kotlin", [
  `-J-Djna.library.path=${join(target, "debug")}`,
  "-classpath",
  `${join(output, "kotlin-smoke.jar")}:${classpath}`,
  "uniffi.isct_ffi_runtime_spike.SmokeKt",
]);
