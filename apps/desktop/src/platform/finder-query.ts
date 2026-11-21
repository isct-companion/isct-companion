import { t } from "../i18n";
import { getDesktopBridge } from "./desktop-bridge";

export type {
  FinderAction,
  FinderPage,
  FinderRowView,
} from "../generated/persistence";
import type { FinderPage } from "../generated/persistence";
import type { FinderRequest } from "./finder-contract";
export type { FinderRequest } from "./finder-contract";

export function finderQuery(request: FinderRequest): Promise<FinderPage> {
  const bridge = getDesktopBridge();
  if (!bridge)
    return Promise.reject(
      new Error(t("finderQuery.courseSearchRequiresTheNative")),
    );
  return bridge.finderQuery(request);
}
