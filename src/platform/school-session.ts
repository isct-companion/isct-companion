import { useEffect, useSyncExternalStore } from "react";

import type {
  DesktopBridge,
  SchoolAuthProfile,
  SchoolSessionEvent,
} from "./desktop-bridge";

/**
 * A school sign-in is only ever working, failed, or not set up. There is no
 * polling: the app signs in once at launch, and afterwards any school page
 * that gets redirected to sign-in is renewed natively with the saved
 * credentials, which reports here through the same session events.
 */
export type SchoolSessionState = "active" | "failed" | "not-configured";

type SessionBridge = Pick<
  DesktopBridge,
  "schoolCredentialStatus" | "ensureSchoolSession" | "onSchoolSession"
>;

// How long one launch sign-in may take before the next account starts.
export const LAUNCH_SIGN_IN_TIMEOUT_MS = 90_000;

const states = new Map<SchoolAuthProfile, SchoolSessionState>();
const listeners = new Set<() => void>();
let attached: SessionBridge | undefined;

export const schoolSessions = {
  get: (profile: SchoolAuthProfile) => states.get(profile),
  set(profile: SchoolAuthProfile, state: SchoolSessionState | undefined) {
    if (states.get(profile) === state) return;
    if (state) states.set(profile, state);
    else states.delete(profile);
    listeners.forEach((listener) => listener());
  },
  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => void listeners.delete(listener);
  },
  /** Forwards native session events into the store; safe to call repeatedly. */
  attach(bridge: SessionBridge | undefined) {
    if (!bridge?.onSchoolSession || attached === bridge) return;
    attached = bridge;
    bridge.onSchoolSession((event: SchoolSessionEvent) =>
      schoolSessions.set(event.profile, event.state),
    );
  },
  /** Test hook. */
  reset() {
    states.clear();
    attached = undefined;
    listeners.forEach((listener) => listener());
  },
};

const settled = (profile: SchoolAuthProfile, timeoutMs: number) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(done, timeoutMs);
    const before = schoolSessions.get(profile);
    const unsubscribe = schoolSessions.subscribe(() => {
      if (schoolSessions.get(profile) !== before) done();
    });
    function done() {
      clearTimeout(timer);
      unsubscribe();
      resolve();
    }
  });

/**
 * Signs every saved account in once, one at a time: they share one school
 * view, so the next account starts only after the previous one settles.
 */
export async function signInAtLaunch(
  bridge: SessionBridge | undefined,
  timeoutMs = LAUNCH_SIGN_IN_TIMEOUT_MS,
) {
  if (!bridge?.ensureSchoolSession) return;
  schoolSessions.attach(bridge);
  const status = await bridge.schoolCredentialStatus().catch(() => undefined);
  if (!status) return;
  const saved: [SchoolAuthProfile, boolean][] = [
    ["science", status.science.saved],
    ["tokyo-tech", status.tokyoTech.saved],
  ];
  for (const [profile, isSaved] of saved) {
    if (!isSaved) {
      schoolSessions.set(profile, "not-configured");
      continue;
    }
    schoolSessions.set(profile, undefined);
    const result = settled(profile, timeoutMs);
    const started = await bridge
      .ensureSchoolSession(profile)
      .catch(() => false);
    if (started) await result;
  }
}

export function useSchoolSession(
  bridge: SessionBridge | undefined,
  profile: SchoolAuthProfile,
) {
  useEffect(() => schoolSessions.attach(bridge), [bridge]);
  return useSyncExternalStore(schoolSessions.subscribe, () =>
    schoolSessions.get(profile),
  );
}
