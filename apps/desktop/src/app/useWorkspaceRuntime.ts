import { useEffect, useRef, useState } from "react";
import type { Workspace } from "../lib/model";
import type { CapabilityRuntime } from "../platform/capabilities";
import type { NativeCapabilitySnapshot } from "../platform/desktop-bridge";
import { bootstrapReminders } from "../platform/reminders";
import { cleanMaterials, retentionDays } from "../features/lms/files";
import { workspaceStore } from "../store/workspace";
import { desktopBridge } from "./native";
import { notify } from "./notify";

export function useWorkspaceRuntime(sourceData: Workspace | undefined) {
  const [schedulerState, setSchedulerState] =
    useState<CapabilityRuntime["background"]["scheduler"]>("off");
  const [nativeCapabilities, setNativeCapabilities] =
    useState<NativeCapabilitySnapshot>();
  const [nativeCapabilitiesReady, setNativeCapabilitiesReady] = useState(false);
  useEffect(() => {
    if (!desktopBridge) return;
    void desktopBridge
      .getRuntimeCapabilities()
      .then(setNativeCapabilities)
      .catch(() => setNativeCapabilities(undefined))
      .finally(() => setNativeCapabilitiesReady(true));
  }, []);
  useEffect(() => {
    void workspaceStore.initialize();
  }, []);

  useEffect(() => {
    if (!sourceData) return;
    const updateState = () => {
      const tasks = workspaceStore.getState().workspace?.scheduledTasks ?? [];
      setSchedulerState(tasks.some((task) => task.enabled) ? "active" : "off");
    };
    updateState();
    const unsubscribe = workspaceStore.subscribe(updateState);
    if (desktopBridge?.runScheduledTask) {
      let cancelled = false;
      let dispose: (() => void) | undefined;
      void desktopBridge
        .onWorkspaceChanged((event) => {
          void workspaceStore.handleWorkspaceChanged(event).catch(notify.error);
        })
        .then((unlisten) => {
          if (cancelled) {
            unlisten();
            return;
          }
          dispose = unlisten;
          // Listening is asynchronous. Recover the snapshot only after actual
          // registration, closing the startup gap between read and subscribe.
          return workspaceStore.reload();
        })
        .catch(notify.error);
      return () => {
        cancelled = true;
        dispose?.();
        unsubscribe();
      };
    }
    // Preview has no runtime; all task admission and timing belong to Core.
    return unsubscribe;
  }, [sourceData?.deviceId]);

  useEffect(() => {
    if (sourceData && desktopBridge)
      void bootstrapReminders(desktopBridge).catch(notify.error);
  }, [sourceData?.deviceId]);

  // Once per launch, after settings load: drop downloaded course files left unused.
  const cleaned = useRef(false);
  useEffect(() => {
    if (!sourceData || cleaned.current) return;
    cleaned.current = true;
    void cleanMaterials(retentionDays(sourceData.settings)).catch(
      () => undefined,
    );
  }, [Boolean(sourceData)]);
  return { schedulerState, nativeCapabilities, nativeCapabilitiesReady };
}
