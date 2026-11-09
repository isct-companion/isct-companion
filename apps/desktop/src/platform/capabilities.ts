import type { MessageKey } from "../i18n";

export type CapabilityState = {
  support: "supported" | "unsupported" | "unknown";
  readiness:
    | "ready"
    | "needs-permission"
    | "needs-configuration"
    | "temporarily-unavailable";
  reason: MessageKey;
};
export type CapabilityId =
  | "local-notifications"
  | "remote-push"
  | "cloud-sync"
  | "background-sync"
  | "site-customization";
export type Platform = "native-desktop" | "browser-preview";
export type CapabilityRuntime = {
  native?: {
    schoolImport: boolean;
    schoolWeb: boolean;
    notifications: boolean;
    notificationUnavailableReason?: MessageKey | null;
    backgroundSchedule: boolean;
    cloudSync: boolean;
  };
  background: { scheduler: "active" | "off" | "failed" };
  noBackendPush: true;
};
export function detectPlatform(): Platform {
  if ("__TAURI_INTERNALS__" in window)
    return window.__ISCT_COMPANION_NATIVE_TARGET__ ?? "native-desktop";
  // Dev-only override so the browser preview can exercise desktop editing:
  //   localStorage.setItem("isct-companion-dev-platform", "native-desktop")
  if (import.meta.env?.DEV) {
    const forced = localStorage.getItem("isct-companion-dev-platform");
    if (forced === "native-desktop") return forced;
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
  runtime?: CapabilityRuntime,
): Record<CapabilityId, CapabilityState> {
  const scheduler = runtime?.background.scheduler ?? "off";
  const native = runtime?.native;
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
              ? "capabilities.permissionIsBlockedInSystem"
              : "capabilities.permissionIsRequestedOnlyWhen",
        }
      : {
          support: "unsupported",
          readiness: "temporarily-unavailable",
          reason:
            native?.notificationUnavailableReason ??
            "capabilities.notificationsAreUnavailableInThis",
        },
    "remote-push": {
      support: "unsupported",
      readiness: "needs-configuration",
      reason: "capabilities.remotePushNeedsAPush",
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
        ? "capabilities.oauthSetupIsPresentSign"
        : "capabilities.addYourOwnGoogleOr",
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
                ? "capabilities.aDesktopScheduleIsActive"
                : scheduler === "failed"
                  ? "capabilities.theConfiguredDesktopScheduleIs"
                  : "capabilities.turnOnATaskIn",
          }
        : {
            support: platform === "native-desktop" ? "unknown" : "unsupported",
            readiness: "temporarily-unavailable",
            reason:
              platform === "native-desktop"
                ? "capabilities.theNativeSchedulerHasNot"
                : "capabilities.mobileLaunchAndManualRefresh",
          },
    "site-customization":
      platform === "native-desktop" && nativeAvailable("schoolWeb")
        ? {
            support: "supported",
            readiness: "needs-permission",
            reason: "capabilities.enableScriptsSeparatelyPerTrusted",
          }
        : {
            support: platform === "native-desktop" ? "unknown" : "unsupported",
            readiness: "temporarily-unavailable",
            reason:
              platform === "native-desktop"
                ? "capabilities.theNativeSchoolWebAdapter"
                : "capabilities.siteCustomizationIsADesktop",
          },
  };
}
