import { ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import type { VpnNotice, VpnStatus } from "../../platform/desktop-bridge";
import { UiButton, UiSelect } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";

/** Codes from the desktop's VPN commands, keyed as the shell reports them. */
const VPN_ERROR_TEXT: Record<string, MessageKey> = {
  helper_not_installed: "tools.campusWifi.vpnHelperNotInstalled",
  helper_outdated: "tools.campusWifi.vpnHelperOutdated",
  helper_unreachable: "tools.campusWifi.vpnHelperUnreachable",
  helper_timeout: "tools.campusWifi.vpnHelperTimeout",
  helper_protocol: "tools.campusWifi.vpnHelperProtocol",
  tunnel_failed: "tools.campusWifi.vpnTunnelFailed",
  vpn_needs_account: "tools.campusWifi.vpnNeedsAccount",
  vpn_login_rejected: "tools.campusWifi.vpnLoginRejected",
  vpn_needs_web: "tools.campusWifi.vpnNeedsWeb",
  vpn_unreachable: "tools.campusWifi.vpnUnreachable",
  vpn_access_denied: "tools.campusWifi.vpnAccessDenied",
  vpn_login_changed: "tools.campusWifi.vpnLoginChanged",
};

const vpnErrorText = (status: VpnStatus) =>
  status.error && VPN_ERROR_TEXT[status.error]
    ? t(VPN_ERROR_TEXT[status.error])
    : (status.detail ?? t("common.signInFailed"));

/** Auto-continue always ends: these are its only choices. */
const KEEP_HOURS = [2, 4, 8, 12, 24];

const clockTime = (at: string) =>
  new Date(at).toLocaleString(undefined, {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

/** Title and body of the system notification for a reminder. */
export const vpnNoticeText = (notice: VpnNotice): [string, string] => {
  switch (notice.kind) {
    case "ending":
      return [
        t("tools.campusWifi.vpnEndingTitle"),
        t("tools.campusWifi.vpnEndingBody", { minutes: notice.minutes }),
      ];
    case "ended":
      return [
        t("tools.campusWifi.vpnEndedTitle"),
        t("tools.campusWifi.vpnEndedBody"),
      ];
    case "keepEnded":
      return [
        t("tools.campusWifi.vpnEndedTitle"),
        t("tools.campusWifi.vpnKeepEndedBody"),
      ];
    case "keepStopped":
      return [
        t("tools.campusWifi.vpnKeepStoppedTitle"),
        vpnErrorText({ state: "error", error: notice.code }),
      ];
  }
};

/** The APM session's countdown, as the webtop shows it, in whole minutes. */
const vpnRemaining = (expiresAt: string | null | undefined, now: number) => {
  const end = expiresAt ? Date.parse(expiresAt) : NaN;
  if (Number.isNaN(end)) return t("tools.campusWifi.vpnConnected");
  const minutes = Math.max(0, Math.floor((end - now) / 60_000));
  return t("tools.campusWifi.vpnRemaining", {
    hours: Math.floor(minutes / 60),
    minutes: minutes % 60,
  });
};

/**
 * The campus VPN: one press signs in to the APM with the saved Tokyo Tech
 * account and starts the tunnel, installing the helper once if it is needed.
 */
export function CampusVpn({ notify }: { notify: (message: string) => void }) {
  const [vpn, setVpn] = useState<VpnStatus>();
  const [vpnBusy, setVpnBusy] = useState(false);
  const vpnConnected = vpn?.state === "connected";
  const [now, setNow] = useState(() => Date.now());

  // The countdown moves by the minute while the tunnel is up.
  useEffect(() => {
    if (!vpnConnected || !vpn?.expiresAt) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [vpnConnected, vpn?.expiresAt]);

  useEffect(() => {
    let current = true;
    void desktopBridge
      ?.vpnStatus?.()
      .catch(() => undefined)
      .then((next) => current && setVpn(next));
    // The minute check reports a tunnel that ended while the page is open.
    const stop = desktopBridge?.onVpnStatus?.((next) => {
      setNow(Date.now());
      setVpn(next);
    });
    return () => {
      current = false;
      stop?.();
    };
  }, []);

  // One press: install the helper once if needed, then Core signs in to the
  // campus VPN with the saved Tokyo Tech account and connects.
  const connectVpn = async () => {
    if (!desktopBridge?.vpnStatus || !desktopBridge.vpnConnect) return;
    setVpnBusy(true);
    try {
      let next = await desktopBridge.vpnStatus();
      if (
        next.error === "helper_not_installed" ||
        next.error === "helper_outdated"
      ) {
        const update = next.error === "helper_outdated";
        const confirmed = await confirmAction(
          t(
            update
              ? "tools.campusWifi.vpnHelperOutdated"
              : "tools.campusWifi.vpnInstallNeeded",
          ),
          t(
            update
              ? "tools.campusWifi.vpnUpdate"
              : "tools.campusWifi.vpnInstall",
          ),
          { title: t("tools.campusWifi.campusVpn"), danger: false },
        );
        if (!confirmed) return;
        next = await desktopBridge.vpnInstall!();
      }
      setVpn(next);
      // Any error here is the last attempt's, which the helper keeps reporting
      // until the next start. `vpn_connect` itself stops on a helper it cannot
      // use, before it spends a login.
      if (next.state === "connected") return;
      setVpn({ state: "connecting" });
      const outcome = await desktopBridge.vpnConnect();
      setNow(Date.now());
      setVpn(outcome);
      if (outcome.error) notify(vpnErrorText(outcome));
    } catch (error) {
      notify(errorText(error) ?? t("common.signInFailed"));
    } finally {
      setVpnBusy(false);
    }
  };

  const disconnectVpn = async () => {
    if (!desktopBridge?.vpnDisconnect) return;
    setVpnBusy(true);
    try {
      setVpn(await desktopBridge.vpnDisconnect());
    } catch (error) {
      notify(errorText(error) ?? t("common.signInFailed"));
    } finally {
      setVpnBusy(false);
    }
  };

  const setKeep = async (hours: number | null) => {
    if (!desktopBridge?.vpnKeep) return;
    try {
      setVpn(await desktopBridge.vpnKeep(hours));
    } catch (error) {
      notify(errorText(error) ?? t("common.signInFailed"));
    }
  };

  const keeping = Boolean(vpn?.keepUntil);
  const vpnHint =
    keeping && !vpnConnected
      ? t("tools.campusWifi.vpnReconnecting")
      : vpn?.error
        ? vpnErrorText(vpn)
        : vpnConnected
          ? vpnRemaining(vpn.expiresAt, now)
          : vpnBusy
            ? t("tools.campusWifi.vpnConnecting")
            : t("tools.campusWifi.vpnDisconnected");

  const unavailable = vpn?.state === "unavailable";
  return (
    <ToolCard
      icon={<ShieldCheck aria-hidden strokeWidth={1.75} />}
      title="tools.campusWifi.campusVpn"
      status={unavailable ? t("tools.campusWifi.vpnUnavailable") : vpnHint}
      note={unavailable ? undefined : "tools.campusWifi.vpnHint"}
      actions={
        !unavailable && (
          <UiButton
            className={`button small${vpnConnected ? " toolConnected" : ""}`}
            disabled={vpnBusy}
            onClick={() =>
              void (vpnConnected || keeping ? disconnectVpn() : connectVpn())
            }
          >
            {t(
              vpnConnected || keeping
                ? "tools.campusWifi.vpnDisconnect"
                : vpn?.error === "helper_not_installed"
                  ? "tools.campusWifi.vpnInstall"
                  : vpn?.error === "helper_outdated"
                    ? "tools.campusWifi.vpnUpdate"
                    : "tools.campusWifi.vpnConnect",
            )}
          </UiButton>
        )
      }
    >
      {(vpnConnected || keeping) &&
        (keeping ? (
          <p className="toolCardNote">
            {vpn?.keepUntil &&
              t("tools.campusWifi.vpnKeepOnHint", {
                until: clockTime(vpn.keepUntil),
              })}{" "}
            <UiButton
              className="button small"
              onClick={() => void setKeep(null)}
            >
              {t("tools.campusWifi.vpnKeepOff")}
            </UiButton>
          </p>
        ) : (
          <UiSelect
            aria-label={t("tools.campusWifi.vpnKeep")}
            placeholder={t("tools.campusWifi.vpnKeepChoose")}
            value={undefined}
            onChange={(value) => void setKeep(Number(value))}
            data={KEEP_HOURS.map((hours) => ({
              value: String(hours),
              label: t("tools.campusWifi.vpnKeepHours", { hours }),
            }))}
          />
        ))}
    </ToolCard>
  );
}
