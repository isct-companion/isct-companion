import { getDesktopBridge } from "./desktop-bridge";

export type {
  FinderAction,
  FinderPage,
  FinderRowView,
} from "../generated/persistence";
import type {
  FinderRequest as CoreFinderRequest,
  FinderPage,
} from "../generated/persistence";

/** Core fills every omitted field with its default (no conditions, the
 * first page of 100 rows). */
type Optional =
  "includeIneligible" | "keys" | "input" | "action" | "offset" | "limit";
export type FinderRequest = Omit<CoreFinderRequest, Optional> &
  Partial<Pick<CoreFinderRequest, Optional>>;

export function finderQuery(request: FinderRequest): Promise<FinderPage> {
  const bridge = getDesktopBridge();
  if (!bridge)
    return Promise.reject(new Error("Course search requires the native Core."));
  return bridge.finderQuery(request);
}
