import { Channel } from "@tauri-apps/api/core";

import { subscribe, type BridgeCommands, type ListenNative } from "./helpers";

export type DesktopTool = "campus-wifi" | "campus-vpn" | "campus-printer";
export type CampusWifiOutcome =
  | "signed-in"
  | "already-online"
  | "needs-account"
  | "rejected"
  | "science-tokyo-x";
export type CampusWifiStatus =
  "signed-in" | "sign-in-required" | "science-tokyo-x" | "other";
/** A campus VPN reminder Core decided is due. */
export type VpnNotice =
  | { kind: "ending"; minutes: number }
  | { kind: "ended" }
  | { kind: "unused" }
  | { kind: "demandStopped"; code: string };
/** Campus VPN as the helper reports it; `error` is a code the UI keys on. */
export type VpnStatus = {
  /** `unavailable`: this development build has no staged VPN payload. */
  state: "disconnected" | "connecting" | "connected" | "error" | "unavailable";
  address?: string | null;
  /** When the campus session ends (RFC 3339), for the countdown. */
  expiresAt?: string | null;
  /** Whether on-demand connecting is on now. */
  onDemand?: boolean;
  /** While on-demand has signed in its hourly most, when it resumes. */
  demandPausedUntil?: string | null;
  /** On the campus network, where the VPN is not needed. */
  onCampus?: boolean;
  /** Publisher sites the tunnel carries, and whether that is the default list. */
  publishers?: string[];
  publishersDefault?: boolean;
  error?: string | null;
  detail?: string | null;
};
/** Sign-in stages in order; "joining" is skipped when already on ScienceTokyo. */
export const CAMPUS_WIFI_STEPS = [
  "joining",
  "opening-portal",
  "signing-in",
  "verifying",
] as const;
export type CampusWifiStep = (typeof CAMPUS_WIFI_STEPS)[number];
/** Whose jobs the campus printer would bill (Core's `PrinterAccount`). */
export type PrinterAccount =
  | { kind: "ready"; studentId: string }
  | { kind: "not-connected" }
  | { kind: "not-student-id" }
  | { kind: "needs-attention" }
  | { kind: "unverified" };
export type CampusPrinterStatus = {
  account: PrinterAccount;
  reach: "reachable" | "unreachable";
};
/** How adding the printer to the system ended. */
type CampusPrinterSetup =
  | { kind: "added" }
  | { kind: "cancelled" }
  | { kind: "account"; account: PrinterAccount }
  | { kind: "failed" };
export interface CampusToolsBridge {
  /** What the Wi-Fi interface is on now (macOS builds only). */
  campusWifiStatus?(): Promise<CampusWifiStatus>;
  /** Campus VPN state from the privileged helper (Apple Silicon and Windows builds). */
  vpnStatus?(): Promise<VpnStatus>;
  /** One administrator approval installs or updates the helper and openconnect. */
  vpnInstall?(): Promise<VpnStatus>;
  /** Signs in to the campus VPN with the saved Tokyo Tech account and connects. */
  vpnConnect?(): Promise<VpnStatus>;
  vpnDisconnect?(): Promise<VpnStatus>;
  /** On-demand connecting on or off for now; the page stores the choice. */
  vpnOnDemand?(enabled: boolean): Promise<VpnStatus>;
  /** The publisher list, or `null` for the default; the page stores it. */
  vpnPublishers?(list: string[] | null): Promise<VpnStatus>;
  /** The state after each check while a tunnel is up (every minute). */
  onVpnStatus?(callback: (status: VpnStatus) => void): () => void;
  /** The session ends in a few minutes, or ended on its own. */
  onVpnNotice?(callback: (notice: VpnNotice) => void): () => void;
  /**
   * Joins ScienceTokyo if needed and signs in once with the Tokyo Tech account
   * (macOS builds only). Returns "science-tokyo-x" without changing anything
   * unless leaving ScienceTokyoX was confirmed.
   */
  campusWifiConnect?(
    leaveScienceTokyoX: boolean,
    onStep: (step: CampusWifiStep) => void,
  ): Promise<CampusWifiOutcome>;
  /** The verified student ID and whether the printer answers, found once. */
  campusPrinterStatus?(): Promise<CampusPrinterStatus>;
  /** Adds or replaces the system printer behind the system's administrator
   * prompt, which shows `prompt` on macOS. */
  installCampusPrinter?(prompt: string): Promise<CampusPrinterSetup>;
}

export function campusToolsCommands(
  { result, invokeCommand }: BridgeCommands,
  listen: ListenNative,
): CampusToolsBridge {
  return {
    campusPrinterStatus: () => result("plugin:campus-printer|status"),
    installCampusPrinter: (prompt) =>
      result("plugin:campus-printer|install", { prompt }),
    campusWifiStatus: () => invokeCommand("plugin:campus-wifi|status"),
    ...(import.meta.env.ISCT_CAMPUS_VPN === false
      ? {}
      : ({
          vpnStatus: () => invokeCommand("plugin:campus-vpn|status"),
          vpnInstall: () => invokeCommand("plugin:campus-vpn|install"),
          vpnConnect: () => invokeCommand("plugin:campus-vpn|connect"),
          vpnDisconnect: () => invokeCommand("plugin:campus-vpn|disconnect"),
          vpnOnDemand: (enabled) =>
            invokeCommand("plugin:campus-vpn|on_demand", { enabled }),
          vpnPublishers: (list) =>
            invokeCommand("plugin:campus-vpn|publishers", { list }),
          onVpnStatus: (callback) =>
            subscribe<VpnStatus>(
              listen,
              "vpn-status",
              (payload) =>
                typeof payload?.state === "string" ? payload : undefined,
              callback,
            ),
          onVpnNotice: (callback) =>
            subscribe<VpnNotice>(
              listen,
              "vpn-notice",
              (payload) =>
                payload?.kind === "ended" ||
                (payload?.kind === "demandStopped" &&
                  typeof payload.code === "string") ||
                (payload?.kind === "ending" &&
                  typeof payload.minutes === "number")
                  ? payload
                  : undefined,
              callback,
            ),
        } satisfies CampusToolsBridge)),
    campusWifiConnect: (leaveScienceTokyoX, onStep) => {
      const steps = new Channel<CampusWifiStep>();
      steps.onmessage = onStep;
      return invokeCommand("plugin:campus-wifi|connect", {
        leaveScienceTokyoX,
        onStep: steps,
      });
    },
  };
}
