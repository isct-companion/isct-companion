import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { nativeOs } from "./desktop-bridge";

type WidgetRoute =
  | { kind: "course"; courseId: string }
  | { kind: "exam"; key: string; term: string; date: string }
  | { kind: "assignment"; id: string; courseId: string | null }
  | {
      kind:
        | "assignments"
        | "timetable"
        | "sync"
        | "unavailableCourse"
        | "unavailableAssignment";
    };

export function onWidgetRoute(route: (destination: WidgetRoute) => void) {
  if (nativeOs() !== "macos") return;
  let active = true;
  const listener = listen<WidgetRoute>("widget-route", (event) => {
    if (active) route(event.payload);
  });
  void listener
    .then(async () => {
      if (!active) return;
      const pending = await invoke<WidgetRoute | null>("widget_frontend_ready");
      if (pending && active) route(pending);
    })
    .catch((error) => console.error("Widget link delivery failed", error));
  return () => {
    active = false;
    void listener.then(
      (dispose) => dispose(),
      () => {},
    );
  };
}
