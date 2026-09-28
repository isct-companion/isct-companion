import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(join(root, path), "utf8");

describe("refactored application architecture", () => {
  it("has no PWA manifest or service worker", () => {
    expect(existsSync(join(root, "public/manifest.webmanifest"))).toBe(false);
    expect(existsSync(join(root, "public/sw.js"))).toBe(false);
    expect(read("index.html")).not.toMatch(
      /manifest\.webmanifest|serviceWorker/i,
    );
    expect(read("src/main.tsx")).not.toMatch(
      /serviceWorker|navigator\.serviceWorker/,
    );
  });

  it("boots Mantine styles and provider", () => {
    expect(read("src/main.tsx")).toContain("@mantine/core/styles.css");
    expect(read("src/app/App.tsx")).toContain("MantineProvider");
    expect(read("src/app/theme.ts")).toContain("cssVariablesResolver");
  });

  it("keeps user CSS and JavaScript customization in Settings", () => {
    const advanced = read("src/pages/settings/AdvancedSettings.tsx");
    expect(advanced).toContain("appCss");
    expect(advanced).toContain("custom.sites[site].css");
    expect(advanced).toContain("custom.sites[site].js");
    const bridge = read("src/platform/desktop-bridge.ts");
    expect(bridge).toContain("css: string");
    expect(bridge).toContain("js: string");
  });

  it("registers Tauri Store as the native shared data layer", () => {
    expect(read("backend/src/lib.rs")).toContain("tauri_plugin_store::Builder");
    expect(read("backend/capabilities/main.json")).toContain("store:default");
    expect(read("src/storage/repository.ts")).toContain(
      'new LazyStore("workspace.json"',
    );
  });

  it("targets the intentionally modern Apple and Android baselines", () => {
    const config = JSON.parse(read("backend/tauri.conf.json"));
    expect(config.bundle.macOS.minimumSystemVersion).toBe("14.0");
    expect(config.bundle.iOS.minimumSystemVersion).toBe("17.0");
    expect(config.bundle.android.minSdkVersion).toBe(34);
  });

  it("keeps login automation script out of the refactored UI layer", () => {
    const script = read("backend/scripts/school-login.js");
    expect(script.length).toBeGreaterThan(1_000);
    expect(read("src/pages/settings/SchoolAccounts.tsx")).not.toContain(
      "school-login.js",
    );
  });

  it("keeps the desktop web toolbar at the 85 / 10 / 5 split", () => {
    const web = read("src/pages/web/WebPage.tsx");
    const styles = read("src/styles.css");
    expect(web).toContain('className="webAddress"');
    expect(web).toContain('className="webZoom"');
    expect(web).toContain('className="iconButton webOpenOutside"');
    expect(styles).toContain(
      "grid-template-columns: minmax(0, 85fr) minmax(0, 10fr) minmax(0, 5fr)",
    );
  });

  it("keeps full-screen workspaces inside the shared 20px safe area", () => {
    expect(read("src/app/App.tsx")).toContain("workspaceMain");
    const styles = read("src/styles.css");
    const workspaceRule = styles.match(/\.workspaceMain\s*\{[^}]*\}/s)?.[0];
    expect(workspaceRule).toContain("padding: 20px");
    expect(workspaceRule).not.toContain("min-height");
    expect(styles).toContain(
      "padding-inline-start: calc(var(--app-shell-navbar-offset, 0rem) + 20px)",
    );
    expect(styles).toMatch(/\.servicesPage\s*\{[^}]*padding:\s*20px/s);
  });
});
