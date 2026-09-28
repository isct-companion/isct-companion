import type { DesktopBridge } from "./desktop-bridge";

/** One authoritative native hide per route departure. */
export function createSchoolWebExit(
  bridge: Pick<DesktopBridge, "hideSchoolWeb"> | undefined,
) {
  let pending: Promise<void> | undefined;
  return {
    get pending() {
      return Boolean(pending);
    },
    close(): Promise<void> {
      if (pending) return pending;
      pending = Promise.resolve()
        .then(async () => {
          if (
            bridge?.hideSchoolWeb &&
            (await bridge.hideSchoolWeb()) !== true
          ) {
            throw new Error(
              "The school page could not be closed. Please retry.",
            );
          }
        })
        .finally(() => {
          pending = undefined;
        });
      return pending;
    },
  };
}
