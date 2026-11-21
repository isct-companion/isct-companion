import { COURSE_FILE_POLICY } from "../generated/persistence";
import type { Settings } from "./model";

export { COURSE_FILE_POLICY };

const boundedInteger = (
  value: number | undefined,
  policy: { default: number; min: number; max: number },
) => {
  const integer = Number.isFinite(value) ? Math.round(value!) : policy.default;
  return Math.min(policy.max, Math.max(policy.min, integer));
};
/** Byte limit for automatic downloads, from the settings value in MB. */
export const downloadLimit = (
  settings: Pick<Settings, "autoDownloadLimitMb">,
) =>
  boundedInteger(
    settings.autoDownloadLimitMb,
    COURSE_FILE_POLICY.autoDownloadLimitMb,
  ) *
  1024 *
  1024;
/** Days a downloaded file may remain unopened before launch cleanup removes it. */
export const retentionDays = (settings: Pick<Settings, "autoCleanDays">) =>
  boundedInteger(settings.autoCleanDays, COURSE_FILE_POLICY.retentionDays);
