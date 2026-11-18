import { copyFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";

const projectTeam = "FB7AL944GK";
const releaseHost = "app.isctcompanion";
const extensionName = "IsctWidgetsExtension";
const plistBuddy = "/usr/libexec/PlistBuddy";
const set = (file, key, value) =>
  execFileSync(plistBuddy, ["-c", `Set :${key} ${value}`, file]);

// Stage the signed extension as a custom Contents file. Tauri copies it
// before signing the containing .app and creating the DMG and update archive.
// `host` is the containing app's identifier: the extension's identifier and
// the widget group follow it, so a development bundle never shares the
// release widgets' data.
export function macWidgets(root, args, macConfig, version, host) {
  const requested =
    args.find((arg) => arg.startsWith("--target="))?.slice(9) ??
    args[args.indexOf("--target") + 1];
  const arch =
    {
      "aarch64-apple-darwin": "arm64",
      "x86_64-apple-darwin": "x86_64",
      "universal-apple-darwin": "arm64 x86_64",
    }[requested] ?? { arm64: "arm64", x64: "x86_64" }[process.arch];
  const widgets = path.join(root, "apps/apple-widgets");
  const desktop = path.join(root, "apps/desktop/src-tauri");
  const output = path.join(root, "target/widgets-mac", arch.replace(" ", "-"));
  mkdirSync(output, { recursive: true });

  const signed =
    process.env.APPLE_SIGNING_IDENTITY ??
    macConfig?.bundle.macOS.signingIdentity;
  const developmentIdentity = args.includes("--debug")
    ? process.env.ISCT_MACOS_WIDGET_DEVELOPMENT_IDENTITY
    : undefined;
  const developmentTeam = process.env.ISCT_MACOS_WIDGET_DEVELOPMENT_TEAM;
  if (developmentIdentity && !developmentTeam) {
    throw new Error(
      "Set ISCT_MACOS_WIDGET_DEVELOPMENT_TEAM for the widget development identity",
    );
  }
  const personal = developmentIdentity && (!signed || signed === "-");
  const team = personal ? developmentTeam : projectTeam;
  const group = `${team}.${host}.widgetkit`;
  const identity = personal ? developmentIdentity : (signed ?? "-");
  const run = (cmd, args) =>
    execFileSync(cmd, args, { cwd: widgets, stdio: "inherit" });
  run("xcodegen", ["generate"]);
  run("xcodebuild", [
    "-project",
    "IsctWidgets.xcodeproj",
    "-scheme",
    extensionName,
    "-configuration",
    "Release",
    "-derivedDataPath",
    output,
    "-destination",
    "generic/platform=macOS",
    `ARCHS=${arch}`,
    "ONLY_ACTIVE_ARCH=NO",
    "CODE_SIGNING_ALLOWED=NO",
    `PRODUCT_BUNDLE_IDENTIFIER=${host}.widgetkit`,
    `ISCT_WIDGET_GROUP=${group}`,
    `MARKETING_VERSION=${version}`,
    `CURRENT_PROJECT_VERSION=${version}`,
    "build",
  ]);
  const appex = path.join(
    output,
    "Build/Products/Release",
    `${extensionName}.appex`,
  );
  const entitlements = path.join(output, "WidgetEntitlements.plist");
  const groupKey = "com.apple.security.application-groups";
  copyFileSync(path.join(widgets, "macOS/Entitlements.plist"), entitlements);
  set(entitlements, `${groupKey}:0`, group);
  run("codesign", [
    "--force",
    "--sign",
    identity,
    "--entitlements",
    entitlements,
    ...(identity === "-"
      ? []
      : [
          "--options",
          "runtime",
          ...(args.includes("--debug") ? [] : ["--timestamp"]),
        ]),
    appex,
  ]);

  const config = { ...macConfig?.bundle.macOS };
  if (personal || host !== releaseHost) {
    const hostInfo = path.join(output, "WidgetHostInfo.plist");
    copyFileSync(path.join(desktop, "Info.plist"), hostInfo);
    set(hostInfo, "IsctWidgetGroup", group);
    config.infoPlist = hostInfo;
  }
  if (personal) {
    // A locally signed bundle shares widget data within this development team.
    // Its profile does not grant the release team's iCloud entitlements.
    const hostEntitlements = path.join(output, "WidgetHostEntitlements.plist");
    copyFileSync(entitlements, hostEntitlements);
    execFileSync(plistBuddy, [
      "-c",
      "Delete :com.apple.security.app-sandbox",
      hostEntitlements,
    ]);
    Object.assign(config, {
      signingIdentity: identity,
      entitlements: hostEntitlements,
    });
  }
  config.files = { ...config.files, [`PlugIns/${extensionName}.appex`]: appex };
  return { bundle: { macOS: config } };
}
