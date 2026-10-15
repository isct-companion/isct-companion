import { join, dirname } from "node:path";
import { mkdirSync, rmSync, copyFileSync, realpathSync } from "node:fs";
import {
  run,
  capture,
  javaDependencies,
  androidJna,
  root,
  output,
} from "./common.mjs";
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
if (!sdk) throw new Error("Set ANDROID_HOME");
const buildTools = process.env.ANDROID_BUILD_TOOLS ?? "36.0.0";
const stage = join(output, "android-smoke");
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
// Test the actual AAR contents, including the native library, on Android ART.
run("unzip", [
  "-q",
  join(root, "bindings/kotlin/artifacts/isct-core.aar"),
  "-d",
  join(stage, "sdk"),
]);
const aar = await androidJna();
run("unzip", ["-q", aar, "-d", join(stage, "jna")]);
const { coroutines } = await javaDependencies();
const kotlinRoot = dirname(
  dirname(realpathSync(capture("which", ["kotlinc"]))),
);
const stdlib = join(kotlinRoot, "lib/kotlin-stdlib.jar");
const alternate = join(kotlinRoot, "libexec/lib/kotlin-stdlib.jar");
const { existsSync } = await import("node:fs");
const kotlinStdlib = existsSync(stdlib) ? stdlib : alternate;
const classes = join(stage, "sdk/classes.jar");
const jna = join(stage, "jna/classes.jar");
run("kotlinc", [
  "crates/isct-ffi/tests/foreign/Smoke.kt",
  "-jvm-target",
  "1.8",
  "-classpath",
  [classes, jna, coroutines].join(":"),
  "-d",
  join(stage, "smoke.jar"),
]);
run(join(sdk, "build-tools", buildTools, "d8"), [
  "--min-api",
  "34",
  "--lib",
  join(sdk, "platforms/android-34/android.jar"),
  "--output",
  join(stage, "smoke.zip"),
  join(stage, "smoke.jar"),
  classes,
  jna,
  coroutines,
  kotlinStdlib,
]);
run("adb", ["wait-for-device"]);
const abi = capture("adb", ["shell", "getprop", "ro.product.cpu.abi"]).trim();
if (!["x86_64", "arm64-v8a"].includes(abi))
  throw new Error(`Unsupported test ABI ${abi}`);
const remote = "/data/local/tmp/isct-sdk-smoke";
run("adb", ["shell", "mkdir", "-p", remote]);
try {
  for (const [local, name] of [
    [join(stage, "smoke.zip"), "smoke.zip"],
    [join(stage, "sdk/jni", abi, "libisct_ffi.so"), "libisct_ffi.so"],
    [join(stage, "jna/jni", abi, "libjnidispatch.so"), "libjnidispatch.so"],
  ])
    run("adb", ["push", local, `${remote}/${name}`]);
  run("adb", [
    "shell",
    "chmod",
    "755",
    `${remote}/libisct_ffi.so`,
    `${remote}/libjnidispatch.so`,
  ]);
  // No UI, network or school account is needed; app_process uses Android's VM and linker.
  const result = capture("adb", [
    "shell",
    `CLASSPATH=${remote}/smoke.zip app_process -Djna.boot.library.path=${remote} -Djna.library.path=${remote} /system/bin app.isctcompanion.core.SmokeKt ${remote}/data`,
  ]);
  console.log(result);
  if (!result.includes("KOTLIN_CORE_SDK_SMOKE_PASSED"))
    throw new Error("Android SDK smoke did not finish");
} finally {
  run("adb", ["shell", "rm", "-rf", remote]);
}
