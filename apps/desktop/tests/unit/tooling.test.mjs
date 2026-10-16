// Build and release tooling: the third-party notices that ship beside the
// binary, the OAuth client configuration the native builds are handed, and the
// generated design tokens. These protect what a release contains, so they drive
// the real generators in `scripts/` and `design/` and read committed files
// through the harness. Plain node: nothing here touches the DOM.
import path from "node:path";
import { describe, expect, it } from "vitest";

import { readRepoFile, repositoryRoot } from "@tests/harness";

import {
  gap,
  invalidated,
  render,
  runSheet,
  validate,
} from "@repo/scripts/checks/manual-acceptance.mjs";
import { parse as parseYaml } from "yaml";
import { staleOutputs } from "@repo/design/build.mjs";
import {
  buildNotices,
  packageRoot,
  renderNotices,
} from "@repo/scripts/desktop/third-party-notices.mjs";
import {
  googleDriveClientId,
  googleRedirectScheme,
  iosInfoPlist,
  nativeOAuthEnv,
  oauthClientTarget,
  readOAuthClients,
} from "@repo/scripts/lib/oauth-config.mjs";

/** A bundled module id, as Vite would report it, for a package in node_modules. */
const bundledModule = (...parts) =>
  path.join(repositoryRoot, "node_modules", ...parts);

/** A repository-relative path, for the harness file readers. */
const rel = (absolute) => path.relative(repositoryRoot, absolute);

describe("third-party notices", () => {
  it("maps a bundled module path to the package root that owns it, scoped or not", () => {
    const nm = `${path.sep}app${path.sep}node_modules${path.sep}`;
    expect(packageRoot(`${nm}react${path.sep}index.js`)).toBe(`${nm}react`);
    expect(
      packageRoot(
        `${nm}@mantine${path.sep}core${path.sep}esm${path.sep}x.mjs?v=1`,
      ),
    ).toBe(`${nm}@mantine${path.sep}core`);
    // Vite reports virtual modules with a leading NUL; the real path is inside.
    expect(packageRoot(`\0${nm}react${path.sep}index.js`)).toBe(`${nm}react`);
    // A first-party module ships under the app's own licence, not a package's.
    expect(packageRoot(`${path.sep}app${path.sep}src${path.sep}main.tsx`)).toBe(
      undefined,
    );
  });

  it("lists the bundled npm packages and the OCR runtime in the notices", () => {
    const notices = buildNotices({
      moduleIds: [
        bundledModule("react", "index.js"),
        bundledModule("react-dom", "index.js"),
      ],
    });
    const names = notices.components.map((item) => item.name);
    expect(names).toEqual(
      expect.arrayContaining(["react", "react-dom", "ONNX Runtime"]),
    );
    // Every component must carry a licence text, or the release ships none.
    for (const item of notices.components) {
      expect(item.texts.length).toBeGreaterThan(0);
      for (const index of item.texts)
        expect(notices.texts[index]).toEqual(expect.any(String));
    }
  });

  it("stores each licence text once", () => {
    const notices = buildNotices({
      moduleIds: [
        bundledModule("react", "index.js"),
        bundledModule("react-dom", "index.js"),
      ],
    });
    expect(new Set(notices.texts).size).toBe(notices.texts.length);
  });

  it("renders each packaged name with the licence text it references", () => {
    const notices = buildNotices({
      moduleIds: [
        bundledModule("react", "index.js"),
        bundledModule("react-dom", "index.js"),
      ],
    });
    const text = renderNotices(notices);
    expect(text).toContain("react-dom");
    expect(text).toContain("Copyright (c) Meta Platforms");
  });
});

describe("OAuth client configuration", () => {
  it("chooses the Google client type from Tauri's target platform", () => {
    expect(oauthClientTarget("ios")).toBe("ios");
    expect(oauthClientTarget(undefined, { PLATFORM_NAME: "iphoneos" })).toBe(
      "ios",
    );
    expect(
      oauthClientTarget(undefined, { SDK_NAME: "iphonesimulator27.0" }),
    ).toBe("ios");
    expect(oauthClientTarget("android")).toBe("android");
    expect(oauthClientTarget("darwin")).toBe("desktop");
    expect(oauthClientTarget(undefined, {})).toBe("desktop");
  });

  it("resolves a platform's client id from the committed OAuth config", () => {
    const committed = readRepoFile("config/oauth-clients.yaml");
    const ios = googleDriveClientId(oauthClientTarget("ios"));
    expect(ios).toMatch(/^\d+-[a-z0-9]+\.apps\.googleusercontent\.com$/);
    expect(committed).toContain(ios);
    expect(googleDriveClientId("desktop")).toMatch(
      /\.apps\.googleusercontent\.com$/,
    );
    expect(googleDriveClientId("android")).toMatch(
      /\.apps\.googleusercontent\.com$/,
    );
  });

  it("treats a platform with no configured client as having no id", () => {
    expect(
      googleDriveClientId("android", { "google-drive": { desktop: "d" } }),
    ).toBe("");
    expect(
      googleDriveClientId("desktop", { "google-drive": { desktop: "  d  " } }),
    ).toBe("d");
  });

  it("reverses a client id into Google's iOS callback scheme", () => {
    expect(googleRedirectScheme("1-ios.apps.googleusercontent.com")).toBe(
      "com.googleusercontent.apps.1-ios",
    );
  });

  it("writes the iOS client id and its callback scheme into the merged Info.plist", () => {
    const clientId = readOAuthClients()["google-drive"].ios;
    const plist = iosInfoPlist(clientId);
    expect(plist).toContain("<key>GIDClientID</key>");
    expect(plist).toContain(`<string>${clientId}</string>`);
    expect(plist).toContain(
      `<string>${googleRedirectScheme(clientId)}</string>`,
    );
  });

  it("hands every platform client id and the desktop secret to the native build", () => {
    const google = readOAuthClients()["google-drive"];
    expect(nativeOAuthEnv(readOAuthClients(), {})).toEqual({
      ISCT_GOOGLE_IOS_CLIENT_ID: google.ios,
      ISCT_GOOGLE_ANDROID_CLIENT_ID: google.android,
      ISCT_GOOGLE_DESKTOP_CLIENT_ID: google.desktop,
      ISCT_GOOGLE_DESKTOP_CLIENT_SECRET: google["desktop-secret"],
    });
  });

  it("lets an already-set environment credential override the committed one", () => {
    const merged = nativeOAuthEnv(readOAuthClients(), {
      ISCT_GOOGLE_DESKTOP_CLIENT_ID: "ci-client-id",
      ISCT_GOOGLE_DESKTOP_CLIENT_SECRET: "ci-secret",
    });
    expect(merged.ISCT_GOOGLE_DESKTOP_CLIENT_ID).toBe("ci-client-id");
    expect(merged.ISCT_GOOGLE_DESKTOP_CLIENT_SECRET).toBe("ci-secret");
  });
});

describe("design tokens", () => {
  it("keeps every committed platform output in step with design/tokens", async () => {
    expect(await staleOutputs()).toEqual([]);
  }, 30_000);
});

// The manual acceptance table records behaviour CI cannot reach. Its results
// are supplied by a person or an agent, so what is enforced here is the
// structure: that the table is well formed, that the rendered document matches
// its source, and that the staleness rule actually fires.
describe("manual acceptance table", () => {
  const source = path.join(
    repositoryRoot,
    "docs/verification/manual-checks.yaml",
  );
  const rendered = path.join(
    repositoryRoot,
    "docs/verification/manual-acceptance.md",
  );
  const load = () => parseYaml(readRepoFile(rel(source)));

  it("is well formed: every step has a reason, persona and observable result", () => {
    expect(validate(load())).toEqual([]);
  });

  it("keeps the rendered document in step with the YAML it is generated from", () => {
    expect(readRepoFile(rel(rendered))).toBe(
      render(load(), { compute: true, skipPending: true }),
    );
  });

  it("rejects a step whose reason is missing, because it hides what it checks", () => {
    const doc = load();
    delete doc.checks[0].steps[0].reason;
    expect(validate(doc)).toContainEqual(
      expect.stringContaining("step 1 has no reason"),
    );
  });

  it("rejects a check that covers only test files, which belongs in tests/", () => {
    const doc = load();
    doc.checks[0].covers = ["tests/**"];
    expect(validate(doc)).toContainEqual(
      expect.stringContaining("automate it instead"),
    );
  });

  it("rejects a recorded result that carries no evidence", () => {
    const doc = load();
    delete doc.checks[0].verified.evidence;
    expect(validate(doc)).toContainEqual(
      expect.stringContaining("must record evidence"),
    );
  });

  it("distinguishes a check invalidated by a change from one never run", () => {
    // A result recorded before the paths it covers last changed is
    // invalidated; a check with no result at all is a gap, not a regression.
    const never = { id: "X", covers: ["README.md"], verified: null };
    expect(invalidated(never, { skipPending: true })).toEqual([]);
    expect(gap(never)).toBe("never verified");

    const stale = {
      id: "Y",
      covers: ["README.md"],
      verified: { commit: "0000000000000000000000000000000000000000" },
    };
    expect(invalidated(stale, { skipPending: true })).toContainEqual(
      expect.stringContaining("changed after"),
    );
    expect(gap(stale)).toBeUndefined();
  });

  it("builds a run sheet that states the persona and why each step exists", () => {
    const doc = load();
    const sheet = runSheet(doc, "SCHOOL-SIGNIN-SCIENCE");
    expect(sheet).toContain(doc.personas[0].label);
    expect(sheet).toContain("Adopt this persona completely");
    for (const step of doc.checks[0].steps) {
      expect(sheet).toContain(step.reason.trim());
    }
    expect(sheet).toContain("--result <pass|fail|unreached>");
  });
});
