import { join } from "node:path";
import {
  mkdirSync,
  copyFileSync,
  rmSync,
  writeFileSync,
  readFileSync,
} from "node:fs";
import { run, generate, target, root, output, generated } from "./common.mjs";
if (process.platform !== "darwin")
  throw new Error("Building XCFrameworks requires Xcode");
const profile = process.env.ISCT_SDK_PROFILE ?? "release";
const features = process.argv.includes("--no-ocr") ? [] : ["--features", "ocr"];
run("cargo", [
  "build",
  "-p",
  "isct-ffi",
  "--features",
  "bindgen",
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
for (const triple of ["aarch64-apple-ios", "aarch64-apple-ios-sim"]) {
  run(
    "cargo",
    [
      "build",
      "-p",
      "isct-ffi",
      "--target",
      triple,
      "--profile",
      profile,
      ...features,
      "--locked",
      "-j",
      "2",
    ],
    {
      env: {
        ...process.env,
        CARGO_TARGET_DIR: target,
        IPHONEOS_DEPLOYMENT_TARGET: "17.0",
      },
    },
  );
  options.push(
    "-library",
    join(
      target,
      triple,
      profile === "dev" ? "debug" : profile,
      "libisct_ffi.a",
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
      ocr: features.length > 0,
    },
    null,
    2,
  ),
);
