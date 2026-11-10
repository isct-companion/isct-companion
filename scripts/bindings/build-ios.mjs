import { join } from "node:path";
import {
  mkdirSync,
  copyFileSync,
  rmSync,
  writeFileSync,
  readFileSync,
} from "node:fs";
import {
  run,
  generate,
  sdkFeatureArgs,
  iosDeploymentTarget,
  target,
  root,
  output,
  generated,
} from "./common.mjs";
if (process.platform !== "darwin")
  throw new Error("Building XCFrameworks requires Xcode");
const profile = process.env.ISCT_SDK_PROFILE ?? "release";
const ocr = !process.argv.includes("--no-ocr");
const bindingFeatures = sdkFeatureArgs({ bindgen: true, ocr });
const targetFeatures = sdkFeatureArgs({ ocr });
run("cargo", [
  "build",
  "-p",
  "app-ffi",
  ...bindingFeatures,
  "--locked",
  "-j",
  "2",
]);
generate();
const headers = join(output, "headers");
mkdirSync(headers, { recursive: true });
copyFileSync(join(generated, "IsctCoreFFI.h"), join(headers, "IsctCoreFFI.h"));
copyFileSync(
  join(generated, "IsctCoreFFI.modulemap"),
  join(headers, "module.modulemap"),
);
const options = [];
// The simulator runs the host's architecture, so an Intel Mac needs the
// x86_64 slice. `--simulator-only` / `--device-only` build one slice for a
// faster local loop.
const simulator =
  process.arch === "x64" ? "x86_64-apple-ios" : "aarch64-apple-ios-sim";
const triples = process.argv.includes("--simulator-only")
  ? [simulator]
  : process.argv.includes("--device-only")
    ? ["aarch64-apple-ios"]
    : ["aarch64-apple-ios", simulator];
for (const triple of triples) {
  run(
    "cargo",
    [
      "build",
      "-p",
      "app-ffi",
      "--target",
      triple,
      "--profile",
      profile,
      ...targetFeatures,
      "--locked",
      "-j",
      "2",
    ],
    {
      env: {
        ...process.env,
        CARGO_TARGET_DIR: target,
        IPHONEOS_DEPLOYMENT_TARGET: iosDeploymentTarget,
      },
    },
  );
  options.push(
    "-library",
    join(
      target,
      triple,
      profile === "dev" ? "debug" : profile,
      "libapp_ffi.a",
    ),
    "-headers",
    headers,
  );
}
const artifacts = join(root, "bindings/swift/artifacts");
mkdirSync(artifacts, { recursive: true });
const framework = join(artifacts, "IsctCore.xcframework");
rmSync(framework, { recursive: true, force: true });
run("xcodebuild", ["-create-xcframework", ...options, "-output", framework]);
copyFileSync(
  join(generated, "IsctCore.swift"),
  join(root, "bindings/swift/Sources/IsctCore/Generated.swift"),
);
writeFileSync(
  join(artifacts, "sdk-build.json"),
  JSON.stringify(
    {
      version: JSON.parse(readFileSync(join(root, "package.json"), "utf8"))
        .version,
      profile,
      ocr,
      minimumIos: iosDeploymentTarget,
    },
    null,
    2,
  ),
);
