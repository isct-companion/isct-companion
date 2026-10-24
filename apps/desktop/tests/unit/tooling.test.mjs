// Build and release tooling: the third-party notices that ship beside the
// binary, the OAuth client configuration the native builds are handed, and the
// generated design tokens. These protect what a release contains, so they drive
// the real generators in `scripts/` and `design/` and read committed files
// through the harness. Plain node: nothing here touches the DOM.
import path from "node:path";
import os from "node:os";
import {
  mkdtempSync,
  writeFileSync,
  readFileSync,
  rmSync,
  mkdirSync,
  cpSync,
  existsSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
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
  nativeOAuthEnv,
  readOAuthClients,
} from "@repo/scripts/lib/oauth-config.mjs";
import { readVersions } from "@repo/scripts/release/versions.mjs";

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
        `${nm}@radix-ui${path.sep}react-dialog${path.sep}esm${path.sep}x.mjs?v=1`,
      ),
    ).toBe(`${nm}@radix-ui${path.sep}react-dialog`);
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
  it("resolves the desktop client id from the committed OAuth config", () => {
    const committed = readRepoFile("config/oauth-clients.yaml");
    const desktop = googleDriveClientId("desktop");
    expect(desktop).toMatch(/^\d+-[a-z0-9]+\.apps\.googleusercontent\.com$/);
    expect(committed).toContain(desktop);
  });

  it("gives the iOS app its own client id and no other platform's", () => {
    const project = readRepoFile("apps/ios/project.yml");
    expect(project).toContain(
      `ISCT_GOOGLE_IOS_CLIENT_ID: ${googleDriveClientId("ios")}`,
    );
    for (const other of ["desktop", "android"]) {
      expect(project).not.toContain(googleDriveClientId(other));
    }
    expect(project).not.toContain("GOCSPX-");
  });

  it("treats a platform with no configured client as having no id", () => {
    expect(
      googleDriveClientId("android", { "google-drive": { desktop: "d" } }),
    ).toBe("");
    expect(
      googleDriveClientId("desktop", { "google-drive": { desktop: "  d  " } }),
    ).toBe("d");
  });

  it("hands the desktop client id and secret to the native build", () => {
    const google = readOAuthClients()["google-drive"];
    expect(nativeOAuthEnv(readOAuthClients(), {})).toEqual({
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
    // Git may check out CRLF on Windows; line endings do not change it.
    expect(readRepoFile(rel(rendered)).replaceAll("\r\n", "\n")).toBe(
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

describe("desktop release publication", () => {
  it.each([
    ["v1.0.0", "--prerelease=false", "--latest=true"],
    ["v1.0.0-rc.4", "--prerelease", "--latest=false"],
  ])(
    "publishes %s with the correct release status",
    (tag, prerelease, latest) => {
      const workflow = parseYaml(
        readRepoFile(".github/workflows/desktop-release.yml"),
      );
      const script = workflow.jobs.publish.steps.find(
        (step) => step.name === "Upload complete release as a draft",
      ).run;
      const dir = mkdtempSync(path.join(os.tmpdir(), "isct-release-"));
      try {
        mkdirSync(path.join(dir, "docs/releases"), { recursive: true });
        writeFileSync(
          path.join(dir, "docs/releases", `${tag.slice(1)}.md`),
          "Reviewed release notes for this version.\n",
        );
        // GitHub has no offline publication mode. Keep the shipped shell script
        // real, replacing only the external CLI so this test never publishes.
        writeFileSync(
          path.join(dir, "gh"),
          String.raw`#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.GH_CALL_LOG, JSON.stringify(args) + '\n');
if (args[0] === 'release' && args[1] === 'view') process.exit(1);
`,
          { mode: 0o755 },
        );
        const log = path.join(dir, "calls.jsonl");
        const result = spawnSync("bash", ["-c", script], {
          cwd: dir,
          encoding: "utf8",
          env: {
            ...process.env,
            PATH: `${dir}${path.delimiter}${process.env.PATH}`,
            GH_CALL_LOG: log,
            RELEASE_TAG: tag,
            GH_REPO: "example/distribution",
            BUILD_REPO: "example/source",
            SOURCE_SHA: "abc123",
            MACOS_SIGNING_STATUS: "signed-notarized",
            WINDOWS_SIGNING_STATUS: "unsigned",
            REPLACE_EXISTING: "false",
          },
        });
        expect(result.status, result.stderr).toBe(0);
        const calls = readFileSync(log, "utf8")
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line));
        const create = calls.find(
          (args) => args[0] === "release" && args[1] === "create",
        );
        const edit = calls.find(
          (args) => args[0] === "release" && args[1] === "edit",
        );
        expect(create).toContain("--draft");
        expect(create).toContain(prerelease);
        expect(edit).toContain(prerelease);
        expect(edit).toContain(latest);
        expect(edit).toContain("--draft=false");
        const upload = calls.findIndex(
          (args) => args[0] === "release" && args[1] === "upload",
        );
        expect(upload).toBeLessThan(calls.indexOf(edit));
        const notes = readFileSync(path.join(dir, "release-notes.md"), "utf8");
        expect(notes).toContain(
          `IsctCompanion ${tag.slice(1)} desktop release`,
        );
        expect(notes).toContain("Android APK is attached by the Android release workflow");
        expect(notes).toContain("Reviewed release notes for this version.");
        // Each platform states its own signing, so a signed Mac build never
        // claims the same for an unsigned Windows installer.
        expect(notes).toContain("notarized by Apple");
        expect(notes).toContain("The Windows installer is unsigned");
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    },
  );
});

describe("desktop update distribution", () => {
  it("pairs native update bundles with signatures and rejects an incomplete signed build", () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "isct-release-"));
    const put = (name, text) => {
      const dest = path.join(root, name);
      mkdirSync(path.dirname(dest), { recursive: true });
      writeFileSync(dest, text);
    };
    try {
      for (const name of [
        "package.json",
        "apps/desktop/package.json",
        "apps/desktop/src-tauri/tauri.conf.json",
        "apps/desktop/src-tauri/Cargo.toml",
        "Cargo.lock",
        "LICENSE",
        "docs/legal/NOTICE.md",
        "docs/legal/NOTICE.ja.md",
        "docs/distribution/README.md",
        "scripts/desktop/desktop-release.mjs",
        "scripts/release/versions.mjs",
      ]) {
        const dest = path.join(root, name);
        mkdirSync(path.dirname(dest), { recursive: true });
        cpSync(path.join(repositoryRoot, name), dest);
      }
      put("apps/desktop/dist/THIRD-PARTY-NOTICES.txt", "Test notices");
      const version = JSON.parse(readRepoFile("package.json")).version;
      put(`docs/releases/${version}.md`, "Reviewed release notes\n");
      const validateTag = (tag) =>
        spawnSync(
          process.execPath,
          [path.join(root, "scripts/desktop/desktop-release.mjs"), "validate"],
          {
            encoding: "utf8",
            env: { ...process.env, GITHUB_REF: `refs/tags/${tag}` },
          },
        );
      expect(validateTag(`v${version}`).status).toBe(0);
      const wrongTag = validateTag("v0.0.0");
      expect(wrongTag.status).not.toBe(0);
      expect(wrongTag.stderr).toContain("Release tag must match app version");
      // The collector packages existing bundles. Native compilation and cryptographic
      // verification belong to Tauri; these bytes are explicitly inert fixtures.
      for (const [target, format, extension, updateFormat, updateExtension] of [
        ["aarch64-apple-darwin", "dmg", ".dmg", "macos", ".app.tar.gz"],
        ["x86_64-apple-darwin", "dmg", ".dmg", "macos", ".app.tar.gz"],
        ["x86_64-pc-windows-msvc", "nsis", ".exe", "nsis", ".exe"],
      ]) {
        put(
          `target/${target}/release/bundle/${format}/fixture${extension}`,
          "test installer",
        );
        const bundle = `target/${target}/release/bundle/${updateFormat}/fixture${updateExtension}`;
        put(bundle, "test update");
        put(`${bundle}.sig`, "test signature");
        const run = () =>
          spawnSync(
            process.execPath,
            [
              path.join(root, "scripts/desktop/desktop-release.mjs"),
              "collect",
              target,
            ],
            {
              encoding: "utf8",
              env: {
                ...process.env,
                GITHUB_REF: "",
                ISCT_UPDATER_PUBLIC_KEY: "test public key",
              },
            },
          );
        expect(run().status).toBe(0);
        const output = path.join(
          root,
          `artifacts/desktop/${target}/IsctCompanion_${version}_${target}${updateExtension}`,
        );
        expect(readFileSync(output, "utf8")).toBe("test update");
        expect(readFileSync(`${output}.sig`, "utf8")).toBe("test signature");
        rmSync(path.join(root, `${bundle}.sig`));
        const missing = run();
        expect(missing.status).not.toBe(0);
        expect(missing.stderr).toContain("missing its signed bundle");
        expect(
          existsSync(path.join(root, "artifacts/desktop", target, "LICENSE")),
        ).toBe(true);
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe("release version bump", () => {
  it("moves every place the release validates, refusing a stale or half-bumped version", () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "isct-bump-"));
    const git = (...args) =>
      spawnSync("git", args, { cwd: root, encoding: "utf8" });
    try {
      for (const name of [
        "package.json",
        "apps/desktop/package.json",
        "apps/desktop/src-tauri/tauri.conf.json",
        "apps/desktop/src-tauri/Cargo.toml",
        "Cargo.lock",
        "LICENSE",
        "docs/legal/NOTICE.md",
        "docs/legal/NOTICE.ja.md",
        "scripts/desktop/desktop-release.mjs",
        "scripts/release/versions.mjs",
        "scripts/release/bump.mjs",
      ]) {
        const dest = path.join(root, name);
        mkdirSync(path.dirname(dest), { recursive: true });
        cpSync(path.join(repositoryRoot, name), dest);
      }
      const version = JSON.parse(readRepoFile("package.json")).version;
      git("init", "-q");
      git("add", ".");
      git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x");
      git("tag", `v${version}`);
      const bump = (next) =>
        spawnSync(
          process.execPath,
          [path.join(root, "scripts/release/bump.mjs"), next],
          { encoding: "utf8" },
        );

      expect(bump(version).stderr).toContain(`not newer than the current`);
      // 1.1.7 shipped its first commit with Cargo.lock left behind: a place out
      // of step stops the bump and is named.
      const lock = path.join(root, "Cargo.lock");
      const locked = readFileSync(lock, "utf8");
      writeFileSync(
        lock,
        locked.replace(
          /(name = "isct-companion"\r?\nversion = ")[^"]+/,
          "$10.0.1",
        ),
      );
      const halfBumped = bump("99.0.0");
      expect(halfBumped.status).not.toBe(0);
      expect(halfBumped.stderr).toContain("Cargo.lock");
      writeFileSync(lock, locked);

      expect(bump("99.0.0").status).toBe(0);
      expect(new Set(readVersions(root).map((place) => place.version))).toEqual(
        new Set(["99.0.0"]),
      );
      const validate = spawnSync(
        process.execPath,
        [path.join(root, "scripts/desktop/desktop-release.mjs"), "validate"],
        { encoding: "utf8", env: { ...process.env, GITHUB_REF: "" } },
      );
      expect(validate.stderr).toBe("");
      expect(validate.status).toBe(0);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
