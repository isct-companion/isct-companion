import { useEffect, useSyncExternalStore } from "react";

import type {
  DesktopBridge,
  SchoolAuthProfile,
  SchoolProfileSession,
  SchoolSessionSnapshot,
} from "./desktop-bridge";

type SessionBridge = Pick<
  DesktopBridge,
  "schoolSessionSnapshot" | "onSchoolSession"
>;

/** Core's latest session snapshot; undefined until the first one arrives. */
let current: SchoolSessionSnapshot | undefined;
const listeners = new Set<() => void>();

type Attachment = {
  bridge: SessionBridge;
  users: number;
  disposed: boolean;
  disposeNative?: () => void;
};

let attached: Attachment | undefined;
let lastBridge: SessionBridge | undefined;

const apply = (snapshot: SchoolSessionSnapshot) => {
  if (current && snapshot.revision <= current.revision) return;
  current = snapshot;
  listeners.forEach((listener) => listener());
};

const stop = (attachment: Attachment) => {
  attachment.disposed = true;
  attachment.disposeNative?.();
  if (attached === attachment) attached = undefined;
};

const start = (attachment: Attachment) => {
  void (async () => {
    const disposeNative = await attachment.bridge.onSchoolSession(
      (snapshot) => {
        if (!attachment.disposed && attached === attachment) apply(snapshot);
      },
    );
    if (attachment.disposed || attached !== attachment) {
      disposeNative();
      return;
    }
    attachment.disposeNative = disposeNative;
    // Listen first, then read: an event during the read wins by revision.
    const snapshot = await attachment.bridge
      .schoolSessionSnapshot()
      .catch(() => undefined);
    if (attachment.disposed || attached !== attachment) return;
    if (snapshot) apply(snapshot);
  })().catch(() => undefined);
};

export const schoolSessions = {
  get: (profile: SchoolAuthProfile): SchoolProfileSession | undefined =>
    profile === "science"
      ? current?.profiles.science
      : current?.profiles.tokyoTech,
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
        const changed = current !== undefined;
        current = undefined;
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
  /** Retry: one foreground sign-in with the saved credentials. The answer is
   * a snapshot, applied like an event from the same bridge. */
  async retry(
    bridge: SessionBridge & Pick<DesktopBridge, "retrySchool">,
    profile: SchoolAuthProfile,
  ) {
    const snapshot = await bridge.retrySchool(profile);
    if (bridge === lastBridge) apply(snapshot);
  },
  /** Test hook. */
  reset() {
    if (attached) stop(attached);
    current = undefined;
    lastBridge = undefined;
    listeners.forEach((listener) => listener());
  },
};

/** The profile's session as Core last reported it; undefined while the
 * first snapshot is out. */
export function useSchoolSession(
  bridge: SessionBridge | undefined,
  profile: SchoolAuthProfile,
) {
  useEffect(() => schoolSessions.attach(bridge), [bridge]);
  return useSyncExternalStore(schoolSessions.subscribe, () =>
    schoolSessions.get(profile),
  );
}
