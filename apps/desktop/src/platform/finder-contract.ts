import type { FinderRequest as CoreFinderRequest } from "../generated/persistence";

/** Core fills every omitted field with its default (no conditions, the
 * first page of 100 rows). */
type Optional =
  "includeIneligible" | "keys" | "input" | "action" | "offset" | "limit";
export type FinderRequest = Omit<CoreFinderRequest, Optional> &
  Partial<Pick<CoreFinderRequest, Optional>>;
