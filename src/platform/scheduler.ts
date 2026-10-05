export type SchedulerRuntimeState = "active" | "off" | "failed";

/** Derive runtime truth from main-process acknowledgements, never saved intent alone. */
export function schedulerStateFromAcknowledgements(
  requested: readonly ("off" | "launch" | "hourly")[],
  acknowledged: readonly boolean[],
): SchedulerRuntimeState {
  if (
    acknowledged.length !== requested.length ||
    acknowledged.some((value) => value !== true)
  ) {
    return "failed";
  }
  return requested.some((value) => value !== "off") ? "active" : "off";
}
