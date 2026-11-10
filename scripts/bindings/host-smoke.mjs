import { join } from "node:path";
import { mkdirSync } from "node:fs";
import {
  run,
  generate,
  javaDependencies,
  target,
  output,
  generated,
  sdkFeatureArgs,
} from "./common.mjs";
const ocr = process.argv.includes("--ocr");
run("cargo", [
  "build",
  "-p",
  "app-ffi",
  ...sdkFeatureArgs({ bindgen: true, ocr }),
  "--locked",
  "-j",
  "2",
]);
generate();
const fixture = "crates/app-ffi/tests/foreign";
if (process.platform === "darwin") {
  run("swiftc", [
    "-emit-module",
    "-emit-library",
    "-module-name",
    "IsctCore",
    "-swift-version",
    "5",
    "-module-cache-path",
    join(output, "ModuleCache"),
    join(generated, "IsctCore.swift"),
    "bindings/swift/Sources/IsctCore/Operation+Value.swift",
    "-Xcc",
    `-fmodule-map-file=${join(generated, "IsctCoreFFI.modulemap")}`,
    "-I",
    generated,
    "-L",
    join(target, "debug"),
    "-lapp_ffi",
    "-emit-module-path",
    join(output, "IsctCore.swiftmodule"),
    "-o",
    join(output, "libIsctCore.dylib"),
  ]);
  run("swiftc", [
    "-parse-as-library",
    "-swift-version",
    "6",
    "-module-cache-path",
    join(output, "ModuleCache"),
    join(fixture, "Smoke.swift"),
    "-Xcc",
    `-fmodule-map-file=${join(generated, "IsctCoreFFI.modulemap")}`,
    "-I",
    generated,
    "-I",
    output,
    "-L",
    output,
    "-lIsctCore",
    "-L",
    join(target, "debug"),
    "-lapp_ffi",
    "-Xlinker",
    "-rpath",
    "-Xlinker",
    output,
    "-Xlinker",
    "-rpath",
    "-Xlinker",
    join(target, "debug"),
    "-o",
    join(output, "swift-smoke"),
  ]);
  run(join(output, "swift-smoke"), []);
}
const host = join(output, "host-generated");
mkdirSync(host, { recursive: true });
run(join(target, "debug/isct-bindgen"), [
  "generate",
  join(
    target,
    "debug",
    process.platform === "darwin" ? "libapp_ffi.dylib" : "libapp_ffi.so",
  ),
  "--config",
  "scripts/bindings/uniffi-host.toml",
  "--language",
  "kotlin",
  "--no-format",
  "--out-dir",
  host,
]);
const { jna, coroutines } = await javaDependencies();
const cp = [jna, coroutines].join(":");
run("kotlinc", [
  join(host, "app/isctcompanion/core/app_ffi.kt"),
  "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/OperationValue.kt",
  join(fixture, "Smoke.kt"),
  "-classpath",
  cp,
  "-d",
  join(output, "kotlin-smoke.jar"),
]);
run("kotlin", [
  `-J-Djna.library.path=${join(target, "debug")}`,
  "-J--enable-native-access=ALL-UNNAMED",
  "-classpath",
  `${join(output, "kotlin-smoke.jar")}:${cp}`,
  "app.isctcompanion.core.SmokeKt",
]);
