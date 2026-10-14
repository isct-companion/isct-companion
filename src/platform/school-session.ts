import { useEffect, useSyncExternalStore } from "react";

import type {
  DesktopBridge,
  SchoolAuthProfile,
  SchoolSessionEvent,
  SchoolSessionSnapshot,
  SchoolSessionState as NativeSchoolSessionState,
} from "./desktop-bridge";

type SessionBridge = Pick<
  DesktopBridge,
  "schoolSessionSnapshot" | "schoolSessionLifecycle" | "onSchoolSession"
>;

export type SchoolSessionState = Exclude<NativeSchoolSessionState, "pending">;

const states = new Map<SchoolAuthProfile, SchoolSessionState>();
const revisions = new Map<SchoolAuthProfile, number>();
const listeners = new Set<() => void>();

type Attachment = {
  bridge: SessionBridge;
  users: number;
  disposed: boolean;
  disposeNative?: () => void;
  disposeLifecycle?: () => void;
};

let attached: Attachment | undefined;
let lastBridge: SessionBridge | undefined;

const visible = () => document.visibilityState !== "hidden";

const apply = (
  profile: SchoolAuthProfile,
  state: NativeSchoolSessionState,
  revision: number,
) => {
  if (revision <= (revisions.get(profile) ?? -1)) return;
  revisions.set(profile, revision);
  const previous = states.get(profile);
  if (state === "pending") states.delete(profile);
  else states.set(profile, state);
  if (previous !== states.get(profile))
    listeners.forEach((listener) => listener());
};

const applyEvent = (event: SchoolSessionEvent) =>
  apply(event.profile, event.state, event.revision);

const applySnapshot = (snapshot: SchoolSessionSnapshot) => {
  apply("science", snapshot.states.science, snapshot.revision);
  apply("tokyo-tech", snapshot.states.tokyoTech, snapshot.revision);
};

const stop = (attachment: Attachment) => {
  attachment.disposed = true;
  attachment.disposeNative?.();
  attachment.disposeLifecycle?.();
  if (attached === attachment) attached = undefined;
};

const start = (attachment: Attachment) => {
  void (async () => {
    const disposeNative = await attachment.bridge.onSchoolSession((event) => {
      if (!attachment.disposed && attached === attachment) applyEvent(event);
    });
    if (attachment.disposed || attached !== attachment) {
      disposeNative();
      return;
    }
    attachment.disposeNative = disposeNative;

    const focus = () => {
      void attachment.bridge
        .schoolSessionLifecycle({ kind: "focus", visible: visible() })
        .catch(() => undefined);
    };
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", focus);
    attachment.disposeLifecycle = () => {
      window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", focus);
    };

    const snapshot = await attachment.bridge
      .schoolSessionSnapshot()
      .catch(() => undefined);
    if (attachment.disposed || attached !== attachment) return;
    if (snapshot) applySnapshot(snapshot);
    await attachment.bridge
      .schoolSessionLifecycle({ kind: "launch", visible: visible() })
      .catch(() => undefined);
  })().catch(() => undefined);
};

export const schoolSessions = {
  get: (profile: SchoolAuthProfile) => states.get(profile),
  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => void listeners.delete(listener);
  },
  attach(bridge: SessionBridge | undefined) {
    if (!bridge) return () => undefined;
    if (attached?.bridge === bridge && !attached.disposed) attached.users += 1;
    else {
      if (attached) stop(attached);
      if (lastBridge && lastBridge !== bridge) {
        const changed = states.size > 0;
        states.clear();
        revisions.clear();
        if (changed) listeners.forEach((listener) => listener());
      }
      lastBridge = bridge;
      attached = { bridge, users: 1, disposed: false };
      start(attached);
    }
    const attachment = attached;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      attachment.users -= 1;
      if (attachment.users === 0) stop(attachment);
    };
  },
  /** Test hook. */
  reset() {
    if (attached) stop(attached);
    states.clear();
    revisions.clear();
    lastBridge = undefined;
    listeners.forEach((listener) => listener());
  },
};

export function useSchoolSession(
  bridge: SessionBridge | undefined,
  profile: SchoolAuthProfile,
) {
  useEffect(() => schoolSessions.attach(bridge), [bridge]);
  return useSyncExternalStore(schoolSessions.subscribe, () =>
    schoolSessions.get(profile),
  );
}
