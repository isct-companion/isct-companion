import { useEffect, useSyncExternalStore } from "react";

import type {
  DesktopBridge,
  SchoolAuthProfile,
  SchoolSessionEvent,
} from "./desktop-bridge";

/**
 * A school sign-in is only ever working, failed, or not set up. There is no
 * polling: the app signs in once at launch and checks again when its window
 * regains focus; in between, any school page that gets redirected to sign-in
 * is renewed natively with the saved credentials. All of it reports here
 * through the same session events.
 */
export type SchoolSessionState = "active" | "failed" | "not-configured";

type SessionBridge = Pick<
  DesktopBridge,
  "schoolCredentialStatus" | "ensureSchoolSession" | "onSchoolSession"
>;

// How long one launch sign-in may take before the next account starts.
export const LAUNCH_SIGN_IN_TIMEOUT_MS = 90_000;
// Returning to the window re-checks at most this often.
export const FOCUS_CHECK_INTERVAL_MS = 2 * 60_000;

// A busy school view (another sign-in, or the person browsing) is retried this often.
const BUSY_RETRIES = 10;
const BUSY_RETRY_MS = 1_500;
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const states = new Map<SchoolAuthProfile, SchoolSessionState>();
const listeners = new Set<() => void>();
// Every native report, even one that repeats the current state.
const reports = new Set<(profile: SchoolAuthProfile) => void>();
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
    bridge.onSchoolSession((event: SchoolSessionEvent) => {
      schoolSessions.set(event.profile, event.state);
      reports.forEach((report) => report(event.profile));
    });
  },
  /** Test hook. */
  reset() {
    states.clear();
    attached = undefined;
    listeners.forEach((listener) => listener());
  },
};

// Resolves on the profile's next report (an unchanged "active" counts) or the timeout.
const settled = (profile: SchoolAuthProfile, timeoutMs: number) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(done, timeoutMs);
    const report = (from: SchoolAuthProfile) => {
      if (from === profile) done();
    };
    reports.add(report);
    function done() {
      clearTimeout(timer);
      reports.delete(report);
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
  quiet = false,
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
    const previous = schoolSessions.get(profile);
    // A focus re-check keeps the current badge until the result arrives.
    if (!quiet) schoolSessions.set(profile, undefined);
    const result = settled(profile, timeoutMs);
    // The shared school view may still be busy with the previous account.
    let started = false;
    for (let attempt = 0; attempt < BUSY_RETRIES && !started; attempt++) {
      if (attempt) await wait(BUSY_RETRY_MS);
      started = await bridge.ensureSchoolSession(profile).catch(() => false);
    }
    if (started) await result;
    else if (!quiet) schoolSessions.set(profile, previous);
  }
}

/**
 * Re-checks saved sessions when the window comes back to the front, so an
 * expired sign-in is renewed before the person opens a school page.
 */
export function checkSessionsOnFocus(
  bridge: SessionBridge | undefined,
  intervalMs = FOCUS_CHECK_INTERVAL_MS,
  now = () => Date.now(),
) {
  if (!bridge?.ensureSchoolSession) return () => undefined;
  let last = now();
  let running = false;
  const check = () => {
    if (running || document.visibilityState === "hidden") return;
    if (now() - last < intervalMs) return;
    last = now();
    running = true;
    void signInAtLaunch(bridge, LAUNCH_SIGN_IN_TIMEOUT_MS, true).finally(() => {
      running = false;
    });
  };
  window.addEventListener("focus", check);
  document.addEventListener("visibilitychange", check);
  return () => {
    window.removeEventListener("focus", check);
    document.removeEventListener("visibilitychange", check);
  };
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
