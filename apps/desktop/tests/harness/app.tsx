import { createElement, StrictMode, type ComponentType } from "react";
import { createHashRouter } from "react-router";
import { installFakeCore, type FakeCore, type FakeCoreOptions } from "./core";
import { mount, type Mounted } from "./render";

/**
 * Mounts the real application over a scripted host.
 *
 * `app/native.ts` captures the bridge once, at first import, and only when
 * `window.__TAURI_INTERNALS__` already exists. Every e2e suite therefore goes
 * through this helper: install the fake host, then import the shell. The bridge
 * keeps delegating to whichever fake is installed, so each suite gets a clean
 * store while the React module graph stays warm.
 *
 * The router is built per mount — the shell caches its default router, which
 * would freeze the first route a suite visited.
 */

type AppRouter = ReturnType<typeof createHashRouter>;
type AppProps = { router?: AppRouter };

export type AppOptions = {
  route?: string;
  strict?: boolean;
};

export type MountedApp = Mounted & {
  core: FakeCore;
  router: AppRouter;
  /** Navigates the real router, then lets React settle. */
  go(route: string): Promise<void>;
};

let shell: Promise<typeof import("@app/app/App")> | undefined;
let routerModule: Promise<typeof import("react-router")> | undefined;
let pathsModule: Promise<typeof import("@app/app/routes")> | undefined;

/**
 * Installs the scripted host and adopts its snapshot without mounting the shell.
 *
 * For suites that mount one production component directly. The shell caches
 * read models by snapshot revision in module scope, so a component suite that
 * skips this step can be served a previous test's answers.
 */
export async function installComponentHost(
  options: FakeCoreOptions,
): Promise<FakeCore> {
  const core = installFakeCore(options);
  await adoptSnapshot();
  return core;
}

async function adoptSnapshot(): Promise<void> {
  const { workspaceStore } = await import("@app/store/workspace");
  await workspaceStore.reload();
}

export async function mountApp(
  options: FakeCoreOptions,
  app: AppOptions = {},
): Promise<MountedApp> {
  const core = installFakeCore(options);
  shell ??= import("@app/app/App");
  routerModule ??= import("react-router");
  pathsModule ??= import("@app/app/routes");
  const [
    { default: App, WorkspaceApp },
    { createHashRouter: buildRouter },
    { paths, workspaceRoutes },
  ] = await Promise.all([shell, routerModule, pathsModule]);

  await adoptSnapshot();

  // The app is browser-stateful; a suite must not inherit the previous one.
  localStorage.clear();
  sessionStorage.clear();
  window.location.hash = app.route ?? paths.home();

  const router = buildRouter(workspaceRoutes(WorkspaceApp));
  const Shell = App as ComponentType<AppProps>;
  const element = createElement(Shell, { router });
  const mounted = await mount(
    app.strict ? createElement(StrictMode, null, element) : element,
  );

  return {
    ...mounted,
    core,
    router,
    async go(route: string) {
      const { act } = await import("react");
      await act(async () => {
        await router.navigate(route);
      });
    },
    async unmount() {
      await mounted.unmount();
      router.dispose();
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    },
  };
}
