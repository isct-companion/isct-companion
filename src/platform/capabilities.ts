export type CapabilityState = {
  support: "supported" | "unsupported" | "unknown";
  readiness:
    | "ready"
    | "needs-permission"
    | "needs-configuration"
    | "temporarily-unavailable";
  reason: string;
};
export type CapabilityId =
  | "local-notifications"
  | "remote-push"
  | "school-web-session"
  | "cloud-sync"
  | "background-sync"
  | "site-customization";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";
export type CapabilityRuntime = {
  native?: {
    schoolImport: boolean;
    schoolWeb: boolean;
    notifications: boolean;
    backgroundSchedule: boolean;
    cloudSync: boolean;
  };
  school: {
    status: "not-configured" | "connected" | "reconnect-required" | "error";
    lastResult?: "fresh" | "cached" | "reconnect-required" | "error";
  };
  background: { scheduler: "active" | "off" | "failed" };
  noBackendPush: true;
};
export function detectPlatform(): Platform {
  if ("__TAURI_INTERNALS__" in window)
    return window.__STWS_NATIVE_TARGET__ ?? "native-desktop";
  // Dev-only override so the browser preview can exercise desktop editing:
  //   localStorage.setItem("stws-dev-platform", "native-desktop")
  if (import.meta.env?.DEV) {
    const forced = localStorage.getItem("stws-dev-platform");
    if (forced === "native-desktop" || forced === "native-mobile")
      return forced;
  }
  return "browser-preview";
}
export function resolveCapabilities(
  platform: Platform,
  cloudConfigured = false,
  notificationPermission: NotificationPermission = typeof Notification ===
  "undefined"
    ? "denied"
    : Notification.permission,
  runtime: CapabilityRuntime | boolean = false,
): Record<CapabilityId, CapabilityState> {
  const school = typeof runtime === "object" ? runtime.school : undefined;
  const schoolReady =
    runtime === true ||
    (school?.status === "connected" && school.lastResult === "fresh");
  const schoolUnavailable =
    school?.status === "error" ||
    school?.lastResult === "cached" ||
    school?.lastResult === "error";
  const schoolExpired =
    school?.status === "reconnect-required" ||
    school?.lastResult === "reconnect-required";
  const scheduler =
    typeof runtime === "object" ? runtime.background.scheduler : "off";
  const native = typeof runtime === "object" ? runtime.native : undefined;
  const nativeAvailable = (
    key: keyof NonNullable<CapabilityRuntime["native"]>,
  ) => native?.[key] === true;
  const notifications =
    platform === "browser-preview"
      ? typeof Notification !== "undefined"
      : nativeAvailable("notifications");
  return {
    "local-notifications": notifications
      ? {
          support: "supported",
          readiness:
            notificationPermission === "granted"
              ? "ready"
              : notificationPermission === "denied"
                ? "temporarily-unavailable"
                : "needs-permission",
          reason:
            notificationPermission === "denied"
              ? "Permission is blocked in system or browser settings."
              : "Permission is requested only when you enable reminders.",
        }
      : {
          support: "unsupported",
          readiness: "temporarily-unavailable",
          reason: "Notifications are unavailable in this runtime.",
        },
    "remote-push": {
      support: "unsupported",
      readiness: "needs-configuration",
      reason:
        "Remote push needs a push service and application server. This no-backend build does not provide one.",
    },
    "school-web-session":
      platform !== "browser-preview" && nativeAvailable("schoolImport")
        ? {
            support: "supported",
            readiness: schoolReady
              ? "ready"
              : schoolUnavailable
                ? "temporarily-unavailable"
                : "needs-configuration",
            reason: schoolReady
              ? "The persistent official-site session has produced a validated import."
              : schoolExpired
                ? "The official site returned a login page. Reconnect once before the next import."
                : schoolUnavailable
                  ? "The last refresh failed or used cached data. Existing records remain available."
                  : "Sign in through an isolated official-site session; no credentials are stored by the app.",
          }
        : {
            support: platform === "browser-preview" ? "unsupported" : "unknown",
            readiness: "temporarily-unavailable",
            reason:
              platform === "browser-preview"
                ? "The browser preview cannot access native school sessions."
                : "The native school adapter has not acknowledged this command.",
          },
    "cloud-sync": {
      support:
        platform === "browser-preview" || nativeAvailable("cloudSync")
          ? "supported"
          : "unknown",
      readiness:
        platform !== "browser-preview" && !nativeAvailable("cloudSync")
          ? "temporarily-unavailable"
          : cloudConfigured
            ? "ready"
            : "needs-configuration",
      reason: cloudConfigured
        ? "OAuth setup is present; sign-in is still required before the first transfer."
        : "Add your own Google or Microsoft OAuth client ID.",
    },
    "background-sync":
      platform === "native-desktop" && nativeAvailable("backgroundSchedule")
        ? {
            support: "supported",
            readiness:
              scheduler === "active"
                ? "ready"
                : scheduler === "failed"
                  ? "temporarily-unavailable"
                  : "needs-configuration",
            reason:
              scheduler === "active"
                ? "A desktop schedule is active while the app is open."
                : scheduler === "failed"
                  ? "The configured desktop schedule is temporarily unavailable."
                  : "Choose a launch or hourly desktop schedule in Advanced.",
          }
        : {
            support: platform === "native-desktop" ? "unknown" : "unsupported",
            readiness: "temporarily-unavailable",
            reason:
              platform === "native-desktop"
                ? "The native scheduler has not acknowledged this command."
                : "Mobile launch and manual refresh are authoritative.",
          },
    "site-customization":
      platform === "native-desktop" && nativeAvailable("schoolWeb")
        ? {
            support: "supported",
            readiness: "needs-permission",
            reason: "Enable scripts separately per trusted origin.",
          }
        : {
            support: platform === "native-desktop" ? "unknown" : "unsupported",
            readiness: "temporarily-unavailable",
            reason:
              platform === "native-desktop"
                ? "The native school web adapter has not acknowledged this command."
                : "Site customization is a desktop-only feature.",
          },
  };
}
export type SchoolImportResult = {
  state: "fresh" | "cached" | "reconnect-required" | "error";
  courses: {
    externalId: string;
    code: string;
    title: string;
    url: string;
    assignments?: {
      externalId: string;
      courseExternalId: string;
      title: string;
      dueAt?: string;
      url: string;
    }[];
    materials?: {
      externalId: string;
      courseExternalId: string;
      title: string;
      url: string;
    }[];
  }[];
  fetchedAt?: string;
  message: string;
};
