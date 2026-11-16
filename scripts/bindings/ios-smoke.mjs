import { join } from "node:path";
import { mkdirSync, writeFileSync, copyFileSync, readFileSync } from "node:fs";
import { run, capture, root, output, iphoneSimulator } from "./common.mjs";
// CI boots the device early with boot-ios-simulator.mjs; this waits for it.
const device = iphoneSimulator();
// A boot the CI step began may still be under way; only a shut-down device
// is booted here.
if (device.state === "Shutdown") run("xcrun", ["simctl", "boot", device.udid]);
run("xcrun", ["simctl", "bootstatus", device.udid, "-b"]);
const sdk = capture("xcrun", ["--sdk", "iphonesimulator", "--show-sdk-path"]);
const fixture = join(output, "ios-package");
mkdirSync(join(fixture, "Sources"), { recursive: true });
copyFileSync(
  join(root, "crates/app-ffi/tests/foreign/Smoke.swift"),
  join(fixture, "Sources/Smoke.swift"),
);
writeFileSync(
  join(fixture, "Package.swift"),
  `// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "SdkSmoke", platforms: [.iOS(.v17)], dependencies: [.package(name: "IsctCoreSDK", path: ${JSON.stringify(join(root, "bindings/swift"))})], targets: [.executableTarget(name: "SdkSmoke", dependencies: [.product(name: "IsctCore", package: "IsctCoreSDK")], path: "Sources")])
`,
);
// Use the shipped package/XCFramework exactly as an application would.
const options = [
  "--package-path",
  fixture,
  "--triple",
  "arm64-apple-ios17.0-simulator",
  "--sdk",
  sdk,
  "--disable-sandbox",
];
run("swift", ["build", ...options]);
const binaries = capture("swift", ["build", ...options, "--show-bin-path"]);
const manifest = JSON.parse(
  readFileSync(join(root, "bindings/swift/artifacts/sdk-build.json"), "utf8"),
);
run("xcrun", [
  "simctl",
  "spawn",
  device.udid,
  join(binaries, "SdkSmoke"),
  ...(manifest.ocr ? ["--require-ocr"] : []),
]);
