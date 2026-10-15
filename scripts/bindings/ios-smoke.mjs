import { join } from "node:path";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { run, capture, root, target, output, generated } from "./common.mjs";
// CI provides an installed runtime. This script never downloads one.
const devices = JSON.parse(
  capture("xcrun", ["simctl", "list", "devices", "available", "--json"]),
).devices;
const device = Object.values(devices)
  .flat()
  .find((d) => d.name.startsWith("iPhone"));
if (!device)
  throw new Error(
    "No available iPhone simulator; install a runtime on the CI runner",
  );
if (device.state !== "Booted") run("xcrun", ["simctl", "boot", device.udid]);
run("xcrun", ["simctl", "bootstatus", device.udid, "-b"]);
const sdk = capture("xcrun", ["--sdk", "iphonesimulator", "--show-sdk-path"]);
const profile = process.env.ISCT_SDK_PROFILE ?? "release";
const library = join(
  target,
  "aarch64-apple-ios-sim",
  profile === "dev" ? "debug" : profile,
);
const build = join(output, "ios-simulator");
mkdirSync(build, { recursive: true });
const flags = [
  "-target",
  "arm64-apple-ios17.0-simulator",
  "-sdk",
  sdk,
  "-module-cache-path",
  join(output, "iOSModuleCache"),
  "-Xcc",
  `-fmodule-map-file=${join(generated, "IsctCoreFFI.modulemap")}`,
  "-I",
  generated,
];
// Compile the same Swift wrapper/source shipped in the package, with a Swift 6 client.
run("xcrun", [
  "swiftc",
  "-emit-module",
  "-emit-library",
  "-module-name",
  "IsctCore",
  "-swift-version",
  "5",
  ...flags,
  join(root, "bindings/swift/Sources/IsctCore/Generated.swift"),
  join(root, "bindings/swift/Sources/IsctCore/Operation+Value.swift"),
  join(library, "libisct_ffi.a"),
  "-framework",
  "Security",
  "-framework",
  "SystemConfiguration",
  "-framework",
  "CoreFoundation",
  "-framework",
  "CoreML",
  "-emit-module-path",
  join(build, "IsctCore.swiftmodule"),
  "-o",
  join(build, "libIsctCore.dylib"),
]);
run("xcrun", [
  "swiftc",
  "-parse-as-library",
  "-swift-version",
  "6",
  ...flags,
  "-I",
  build,
  "-L",
  build,
  "-lIsctCore",
  join(root, "crates/isct-ffi/tests/foreign/Smoke.swift"),
  "-o",
  join(build, "smoke"),
]);
// simctl spawn runs a simulator-target executable without creating a product UI.
run("xcrun", ["simctl", "spawn", device.udid, join(build, "smoke")], {
  env: { ...process.env, SIMCTL_CHILD_DYLD_LIBRARY_PATH: build },
});
