import { t } from "../i18n";
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
    settled(): Promise<void> {
      return pending ?? Promise.resolve();
    },
    close(): Promise<void> {
      if (pending) return pending;
      pending = Promise.resolve()
        .then(async () => {
          if (
            bridge?.hideSchoolWeb &&
            (await bridge.hideSchoolWeb()) !== true
          ) {
            throw new Error(t("schoolWebExit.theSchoolPageCouldNot"));
          }
        })
        .finally(() => {
          pending = undefined;
        });
      return pending;
    },
  };
}
