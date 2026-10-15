import { join, dirname } from "node:path";
import {
  mkdirSync,
  copyFileSync,
  rmSync,
  writeFileSync,
  readdirSync,
  readFileSync,
} from "node:fs";
import {
  run,
  generate,
  javaDependencies,
  androidJna,
  capture,
  root,
  target,
  output,
  generated,
} from "./common.mjs";
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT;
const ndk = process.env.NDK_HOME;
if (!sdk || !ndk) throw new Error("Set ANDROID_HOME and NDK_HOME");
const profile = process.env.ISCT_SDK_PROFILE ?? "release";
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
const host = process.platform === "darwin" ? "darwin-x86_64" : "linux-x86_64";
const toolchain = join(ndk, "toolchains/llvm/prebuilt", host, "bin");
const features = process.argv.includes("--no-ocr") ? [] : ["--features", "ocr"];
const stage = join(output, "android-aar");
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
for (const [triple, abi, clang] of [
  ["aarch64-linux-android", "arm64-v8a", "aarch64-linux-android34-clang"],
  ["x86_64-linux-android", "x86_64", "x86_64-linux-android34-clang"],
]) {
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: target,
    [`CARGO_TARGET_${triple.replaceAll("-", "_").toUpperCase()}_LINKER`]: join(
      toolchain,
      clang,
    ),
    [`CC_${triple.replaceAll("-", "_")}`]: join(toolchain, clang),
    [`AR_${triple.replaceAll("-", "_")}`]: join(toolchain, "llvm-ar"),
  };
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
    { env },
  );
  const dir = join(stage, "jni", abi);
  mkdirSync(dir, { recursive: true });
  copyFileSync(
    join(
      target,
      triple,
      profile === "dev" ? "debug" : profile,
      "libisct_ffi.so",
    ),
    join(dir, "libisct_ffi.so"),
  );
  const needed = capture(join(toolchain, "llvm-readelf"), [
    "-d",
    join(dir, "libisct_ffi.so"),
  ]);
  if (needed.includes("libc++_shared.so"))
    copyFileSync(
      join(toolchain, "../sysroot/usr/lib", triple, "libc++_shared.so"),
      join(dir, "libc++_shared.so"),
    );
}
const { jna, coroutines } = await javaDependencies();
const android = join(sdk, "platforms/android-34/android.jar");
run("kotlinc", [
  join(generated, "app/isctcompanion/core/isct_ffi.kt"),
  "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/OperationValue.kt",
  "-classpath",
  [jna, coroutines, android].join(":"),
  "-jvm-target",
  "1.8",
  "-d",
  join(stage, "classes.jar"),
]);
writeFileSync(
  join(stage, "AndroidManifest.xml"),
  '<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.isctcompanion.core"><uses-sdk android:minSdkVersion="34"/></manifest>',
);
writeFileSync(
  join(stage, "consumer-rules.pro"),
  "-keep class com.sun.jna.** { *; }\n-keep class app.isctcompanion.core.** { *; }\n",
);
writeFileSync(join(stage, "R.txt"), "");
const artifacts = join(root, "bindings/kotlin/artifacts");
mkdirSync(artifacts, { recursive: true });
const aar = join(artifacts, "isct-core.aar");
rmSync(aar, { force: true });
run("zip", ["-qr", aar, "."], { cwd: stage });
copyFileSync(
  join(generated, "app/isctcompanion/core/isct_ffi.kt"),
  join(
    root,
    "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/isct_ffi.kt",
  ),
);
const version = JSON.parse(
  readFileSync(join(root, "package.json"), "utf8"),
).version;
writeFileSync(
  join(artifacts, "isct-core.pom"),
  `<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>app.isctcompanion</groupId><artifactId>isct-core</artifactId><version>${version}</version><packaging>aar</packaging><dependencies><dependency><groupId>net.java.dev.jna</groupId><artifactId>jna</artifactId><version>5.17.0</version><type>aar</type></dependency><dependency><groupId>org.jetbrains.kotlinx</groupId><artifactId>kotlinx-coroutines-android</artifactId><version>1.10.2</version></dependency></dependencies></project>`,
);
await androidJna();
writeFileSync(
  join(artifacts, "sdk-build.json"),
  JSON.stringify(
    {
      version,
      profile,
      ocr: features.length > 0,
      abis: ["arm64-v8a", "x86_64"],
    },
    null,
    2,
  ),
);
