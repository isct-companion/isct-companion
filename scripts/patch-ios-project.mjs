import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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

function patchEntitlements({ required }) {
  let source;
  try {
    source = readFileSync(entitlements, "utf8");
  } catch (error) {
    if (!required && error?.code === "ENOENT") return false;
    throw error;
  }
  if (source.includes("<key>keychain-access-groups</key>")) return false;
  const insertion =
    "\t<key>keychain-access-groups</key>\n" +
    "\t<array>\n" +
    "\t\t<string>$(AppIdentifierPrefix)app.isctcompanion</string>\n" +
    "\t</array>\n";
  const patched = source.replace("</dict>", `${insertion}</dict>`);
  if (patched === source) throw new Error("Could not patch iOS entitlements.");
  writeFileSync(entitlements, patched);
  return true;
}

export function patchIosProject({ required = true } = {}) {
  let changed = patchEntitlements({ required });
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
