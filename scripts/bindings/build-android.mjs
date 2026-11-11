import { join, delimiter } from "node:path";
import {
  mkdirSync,
  copyFileSync,
  rmSync,
  writeFileSync,
  readFileSync,
  existsSync,
} from "node:fs";
import {
  run,
  generate,
  javaDependencies,
  androidJna,
  androidAnnotations,
  androidCompileSdk,
  androidMinSdk,
  capture,
  kotlinc,
  kotlincVersion,
  sdkFeatureArgs,
  root,
  target,
  output,
  generated,
} from "./common.mjs";
const app = join(root, "apps/android");
// Without the variables, the SDK Android Studio recorded for the app and the
// NDK version the app pins, so a configured checkout builds with no setup.
const localSdk = existsSync(join(app, "local.properties"))
  ? readFileSync(join(app, "local.properties"), "utf8")
      .match(/^sdk\.dir=(.+)$/m)?.[1]
      .trim()
      .replace(/\\(.)/g, "$1")
  : undefined;
const sdk =
  process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT ?? localSdk;
const ndkVersion = readFileSync(
  join(app, "app/build.gradle.kts"),
  "utf8",
).match(/ndkVersion = "([^"]+)"/)?.[1];
const ndk =
  process.env.NDK_HOME ?? (sdk && ndkVersion && join(sdk, "ndk", ndkVersion));
if (!sdk || !ndk || !existsSync(ndk))
  throw new Error("Set ANDROID_HOME and NDK_HOME");
const assemble = process.argv.includes("--assemble-debug");
// The app ships arm64 only. `--with-emulator` adds x86_64 so the SDK can be
// loaded on an x86_64 emulator (the CI smoke test).
const emulator = process.argv.includes("--with-emulator");
const profile = process.env.ISCT_SDK_PROFILE ?? "release";
const ocr = !process.argv.includes("--no-ocr");
// The bindings are generated from the same features the library has, so the
// campus map's records and calls are in the Kotlin SDK.
const bindingFeatures = sdkFeatureArgs({ bindgen: true, ocr, campusMap: true });
const targetFeatures = sdkFeatureArgs({ ocr, campusMap: true });
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
const windows = process.platform === "win32";
const host =
  { darwin: "darwin-x86_64", win32: "windows-x86_64" }[process.platform] ??
  "linux-x86_64";
const toolchain = join(ndk, "toolchains/llvm/prebuilt", host, "bin");
// The NDK's per-API clang wrappers are batch files on Windows.
const wrapper = windows ? ".cmd" : "";
const exe = windows ? ".exe" : "";
const stage = join(output, "android-aar");
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
const abis = [
  [
    "aarch64-linux-android",
    "arm64-v8a",
    `aarch64-linux-android${androidMinSdk}-clang`,
  ],
  ...(emulator
    ? [
        [
          "x86_64-linux-android",
          "x86_64",
          `x86_64-linux-android${androidMinSdk}-clang`,
        ],
      ]
    : []),
];
for (const [triple, abi, clang] of abis) {
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: target,
    [`CARGO_TARGET_${triple.replaceAll("-", "_").toUpperCase()}_LINKER`]: join(
      toolchain,
      clang + wrapper,
    ),
    [`CC_${triple.replaceAll("-", "_")}`]: join(toolchain, clang + wrapper),
    [`AR_${triple.replaceAll("-", "_")}`]: join(toolchain, "llvm-ar" + exe),
    // Packed relative relocations (RELR, API 30+; the SDK floor is 34) shrink
    // the arm64 library's 2.3 MB relocation table to about 40 KB.
    [`CARGO_TARGET_${triple.replaceAll("-", "_").toUpperCase()}_RUSTFLAGS`]:
      "-C link-arg=-Wl,-z,pack-relative-relocs",
  };
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
    { env },
  );
  const dir = join(stage, "jni", abi);
  mkdirSync(dir, { recursive: true });
  copyFileSync(
    join(
      target,
      triple,
      profile === "dev" ? "debug" : profile,
      "libapp_ffi.so",
    ),
    join(dir, "libapp_ffi.so"),
  );
  const needed = capture(join(toolchain, "llvm-readelf" + exe), [
    "-d",
    join(dir, "libapp_ffi.so"),
  ]);
  if (needed.includes("libc++_shared.so"))
    copyFileSync(
      join(toolchain, "../sysroot/usr/lib", triple, "libc++_shared.so"),
      join(dir, "libc++_shared.so"),
    );
}
const { jna, coroutines } = await javaDependencies();
const annotations = await androidAnnotations();
const android = join(sdk, `platforms/android-${androidCompileSdk}/android.jar`);
// The app compiles against these classes with its own Kotlin plugin, which
// rejects metadata more than one minor version newer. A host kotlinc newer
// than the app's (CI runners ship one) emits the app's language version; an
// older one cannot name it and emits its own, which the app reads anyway.
const appKotlin = readFileSync(
  join(root, "apps/android/build.gradle.kts"),
  "utf8",
).match(/id\("org\.jetbrains\.kotlin\.android"\) version "(\d+\.\d+)/)?.[1];
if (!appKotlin)
  throw new Error("Missing the Kotlin plugin version in apps/android");
const hostKotlin = kotlincVersion();
const [appMajor, appMinor] = appKotlin.split(".").map(Number);
const [hostMajor, hostMinor] = hostKotlin.split(".").map(Number);
const emitKotlin =
  hostMajor < appMajor || (hostMajor === appMajor && hostMinor < appMinor)
    ? hostKotlin
    : appKotlin;
kotlinc([
  join(generated, "app/isctcompanion/core/app_ffi.kt"),
  "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/OperationValue.kt",
  "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/TlsVerifier.kt",
  "-classpath",
  [jna, coroutines, android, annotations].join(delimiter),
  "-language-version",
  emitKotlin,
  "-api-version",
  emitKotlin,
  "-jvm-target",
  "1.8",
  "-d",
  join(stage, "classes.jar"),
]);
writeFileSync(
  join(stage, "AndroidManifest.xml"),
  `<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.isctcompanion.core"><uses-sdk android:minSdkVersion="${androidMinSdk}"/></manifest>`,
);
writeFileSync(
  join(stage, "proguard.txt"),
  "-keep class com.sun.jna.** { *; }\n-keep class app.isctcompanion.core.** { *; }\n",
);
writeFileSync(join(stage, "R.txt"), "");
const artifacts = join(root, "bindings/kotlin/artifacts");
mkdirSync(artifacts, { recursive: true });
const aar = join(artifacts, "app-core.aar");
rmSync(aar, { force: true });
const entries = [
  "AndroidManifest.xml",
  "classes.jar",
  "proguard.txt",
  "R.txt",
  "jni",
];
// Windows has no zip; its bundled bsdtar (not Git's GNU tar) writes one.
if (windows)
  run(
    join(process.env.SystemRoot, "System32/tar.exe"),
    ["--format", "zip", "-cf", aar, ...entries],
    { cwd: stage },
  );
else run("zip", ["-qr", aar, ...entries], { cwd: stage });
copyFileSync(
  join(generated, "app/isctcompanion/core/app_ffi.kt"),
  join(
    root,
    "bindings/kotlin/src/main/kotlin/app/isctcompanion/core/app_ffi.kt",
  ),
);
const version = JSON.parse(
  readFileSync(join(root, "package.json"), "utf8"),
).version;
writeFileSync(
  join(artifacts, "app-core.pom"),
  `<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>app.isctcompanion</groupId><artifactId>app-core</artifactId><version>${version}</version><packaging>aar</packaging><dependencies><dependency><groupId>net.java.dev.jna</groupId><artifactId>jna</artifactId><version>5.17.0</version><type>aar</type></dependency><dependency><groupId>org.jetbrains.kotlinx</groupId><artifactId>kotlinx-coroutines-android</artifactId><version>1.10.2</version></dependency><dependency><groupId>androidx.annotation</groupId><artifactId>annotation-jvm</artifactId><version>1.9.1</version></dependency>${
    // The OCR build opens LiteRT's native library at run time.
    ocr
      ? "<dependency><groupId>com.google.ai.edge.litert</groupId><artifactId>litert</artifactId><version>2.3.2</version></dependency>"
      : ""
  }</dependencies></project>`,
);
await androidJna();
writeFileSync(
  join(artifacts, "sdk-build.json"),
  JSON.stringify(
    {
      version,
      profile,
      ocr,
      minSdk: androidMinSdk,
      compileSdk: androidCompileSdk,
      abis: abis.map(([, abi]) => abi),
    },
    null,
    2,
  ),
);
// One command from a clean checkout to the debug APK (`yarn android:debug`).
if (assemble)
  // A full path: cmd.exe may be told not to search the current directory.
  run(
    windows ? `"${join(app, "gradlew.bat")}"` : join(app, "gradlew"),
    [":app:assembleDebug"],
    {
      cwd: app,
      shell: windows,
    },
  );
