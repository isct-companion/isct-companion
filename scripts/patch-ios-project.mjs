import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const project = fileURLToPath(
  new URL(
    "../backend/gen/apple/isct-companion.xcodeproj/project.pbxproj",
    import.meta.url,
  ),
);
const entitlements = fileURLToPath(
  new URL(
    "../backend/gen/apple/isct-companion_iOS/isct-companion_iOS.entitlements",
    import.meta.url,
  ),
);

export function patchIosEntitlements(file = entitlements, { required = true } = {}) {
  try {
    readFileSync(file);
  } catch (error) {
    if (!required && error?.code === "ENOENT") return false;
    throw error;
  }
  // plutil handles the empty and binary plists emitted by a fresh Tauri init.
  const parsed = spawnSync("plutil", ["-convert", "json", "-o", "-", file], { encoding: "utf8" });
  if (parsed.status !== 0) throw new Error(`Could not read iOS entitlements: ${parsed.stderr}`);
  const original = JSON.parse(parsed.stdout);
  const patched = { ...original,
    "com.apple.developer.icloud-container-identifiers": ["iCloud.app.isctcompanion"],
    "com.apple.developer.icloud-services": ["CloudKit"],
    "keychain-access-groups": ["$(AppIdentifierPrefix)app.isctcompanion"],
  };
  if (JSON.stringify(original) === JSON.stringify(patched)) return false;
  const written = spawnSync("plutil", ["-convert", "xml1", "-o", file, "-"], { input: JSON.stringify(patched), encoding: "utf8" });
  if (written.status !== 0) throw new Error(`Could not write iOS entitlements: ${written.stderr}`);
  return true;
}

export function patchIosProject({ required = true } = {}) {
  let changed = patchIosEntitlements(entitlements, { required });
  let source;
  try {
    source = readFileSync(project, "utf8");
  } catch (error) {
    if (!required && error?.code === "ENOENT") return changed;
    throw error;
  }
  const appAuthLinkerFlag =
    'OTHER_LDFLAGS = "$(inherited) -Wl,-u,_OBJC_CLASS_$_OIDExternalUserAgentIOS";';
  if (!source.includes(appAuthLinkerFlag)) {
    source = source.replaceAll(
      'OTHER_LDFLAGS = "$(inherited) -ObjC";',
      appAuthLinkerFlag,
    );
    if (!source.includes(appAuthLinkerFlag)) {
      const marker = "\t\t\t\tALWAYS_EMBED_SWIFT_STANDARD_LIBRARIES = YES;";
      const matches = source.split(marker).length - 1;
      if (matches !== 2) {
        throw new Error(
          `Expected two iOS target build configurations, found ${matches}.`,
        );
      }
      source = source.replaceAll(marker, `${marker}\n\t\t\t\t${appAuthLinkerFlag}`);
    }
    writeFileSync(project, source);
    changed = true;
  }
  return changed;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) patchIosProject();
