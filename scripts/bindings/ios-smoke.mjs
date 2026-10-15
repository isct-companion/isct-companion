import { join } from "node:path";
import { mkdirSync, writeFileSync, copyFileSync, readFileSync } from "node:fs";
import { run, capture, root, output } from "./common.mjs";
// CI supplies the runtime. No simulator download occurs on a developer's host.
const devices = JSON.parse(
  capture("xcrun", ["simctl", "list", "devices", "available", "--json"]),
).devices;
const device = Object.values(devices)
  .flat()
  .find((d) => d.name.startsWith("iPhone"));
if (!device) throw new Error("No installed iPhone simulator runtime");
if (device.state !== "Booted") run("xcrun", ["simctl", "boot", device.udid]);
run("xcrun", ["simctl", "bootstatus", device.udid, "-b"]);
const sdk = capture("xcrun", ["--sdk", "iphonesimulator", "--show-sdk-path"]);
const fixture = join(output, "ios-package");
mkdirSync(join(fixture, "Sources"), { recursive: true });
copyFileSync(
  join(root, "crates/isct-ffi/tests/foreign/Smoke.swift"),
  join(fixture, "Sources/Smoke.swift"),
);
writeFileSync(
  join(fixture, "Package.swift"),
  `// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "SdkSmoke", platforms: [.iOS(.v17)], dependencies: [.package(path: ${JSON.stringify(join(root, "bindings/swift"))})], targets: [.executableTarget(name: "SdkSmoke", dependencies: [.product(name: "IsctCore", package: "swift")], path: "Sources")])
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
