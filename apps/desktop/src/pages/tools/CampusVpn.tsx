import { ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import type { VpnNotice, VpnStatus } from "../../platform/desktop-bridge";
import { InfoTip, Switch, UiButton } from "../../components/ui";
import { workspaceActions } from "../../store/workspace";
import { confirmAction } from "../../components/confirm";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";
import { jstClock } from "../../lib/civil-date";

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
    case "unused":
      return [
        t("tools.campusWifi.vpnUnusedTitle"),
        t("tools.campusWifi.vpnUnusedBody"),
      ];
    case "demandStopped":
      return [
        t("tools.campusWifi.vpnDemandStoppedTitle"),
        vpnErrorText({ state: "error", error: notice.code }),
      ];
  }
};

/** This device's on-demand choice, which the next launch reads. */
export const saveVpnOnDemand = (enabled: boolean) =>
  workspaceActions.updateSettings((draft) => {
    draft.settings.campusVpnOnDemand = enabled;
    return draft;
  });

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

  // The helper, installed or updated once with the person's consent; the
  // status after it, or nothing when they declined.
  const readyHelper = async () => {
    if (!desktopBridge?.vpnStatus) return;
    const next = await desktopBridge.vpnStatus();
    if (
      next.error !== "helper_not_installed" &&
      next.error !== "helper_outdated"
    )
      return next;
    const update = next.error === "helper_outdated";
    const confirmed = await confirmAction(
      t(
        update
          ? "tools.campusWifi.vpnHelperOutdated"
          : "tools.campusWifi.vpnInstallNeeded",
      ),
      t(update ? "tools.campusWifi.vpnUpdate" : "tools.campusWifi.vpnInstall"),
      { title: t("tools.campusWifi.campusVpn"), danger: false },
    );
    return confirmed ? desktopBridge.vpnInstall!() : undefined;
  };

  // One press: install the helper once if needed, then Core signs in to the
  // campus VPN with the saved Tokyo Tech account and connects.
  const connectVpn = async () => {
    if (!desktopBridge?.vpnConnect) return;
    setVpnBusy(true);
    try {
      const next = await readyHelper();
      if (!next) return;
      setVpn(next);
      // Any error here is the last attempt's, which the helper keeps reporting
      // until the next start. `vpn_connect` itself stops on a helper it cannot
      // use, before it spends a login.
      if (next.state === "connected") return;
      // On campus the school's network reaches campus hosts already; a
      // tunnel would only take one of its few VPN ports.
      if (
        next.onCampus &&
        !(await confirmAction(
          t("tools.campusWifi.vpnOnCampusBody"),
          t("tools.campusWifi.vpnConnectAnyway"),
          { title: t("tools.campusWifi.vpnOnCampusTitle"), danger: false },
        ))
      )
        return;
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

  // Installs or updates the helper only: automatic management connects by
  // itself once it is there.
  const installHelper = async () => {
    setVpnBusy(true);
    try {
      const next = await readyHelper();
      if (next) setVpn(next);
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

  // Standby needs the current helper, as a connect does.
  const setOnDemand = async (enabled: boolean) => {
    if (!desktopBridge?.vpnOnDemand) return;
    setVpnBusy(true);
    try {
      if (enabled && !(await readyHelper())) return;
      setVpn(await desktopBridge.vpnOnDemand(enabled));
      await saveVpnOnDemand(enabled);
    } catch (error) {
      notify(errorText(error) ?? t("common.signInFailed"));
    } finally {
      setVpnBusy(false);
    }
  };

  const vpnHint = vpn?.error
    ? vpnErrorText(vpn)
    : vpnConnected
      ? vpnRemaining(vpn.expiresAt, now)
      : vpnBusy || vpn?.state === "connecting"
        ? t("tools.campusWifi.vpnConnecting")
        : vpn?.onCampus
          ? t("tools.campusWifi.vpnOnCampus")
          : vpn?.demandPausedUntil
            ? t("tools.campusWifi.vpnPaused", {
                time: jstClock(new Date(vpn.demandPausedUntil)),
              })
            : vpn?.onDemand
              ? t("tools.campusWifi.vpnStandby")
              : t("tools.campusWifi.vpnDisconnected");

  const unavailable = vpn?.state === "unavailable";
  const helperNeeded =
    vpn?.error === "helper_not_installed" || vpn?.error === "helper_outdated";
  // Automatic management connects and disconnects by itself: a press beside
  // it would only be undone by the next use, so it shows none. The helper
  // still needs the person's consent to install, in either mode.
  const action = helperNeeded ? (
    <UiButton
      className="button small"
      disabled={vpnBusy}
      onClick={() => void installHelper()}
    >
      {t(
        vpn?.error === "helper_outdated"
          ? "tools.campusWifi.vpnUpdate"
          : "tools.campusWifi.vpnInstall",
      )}
    </UiButton>
  ) : vpn?.onDemand ? null : (
    <UiButton
      className={`button small${vpnConnected ? " toolConnected" : ""}`}
      disabled={vpnBusy}
      onClick={() => void (vpnConnected ? disconnectVpn() : connectVpn())}
    >
      {t(
        vpnConnected
          ? "tools.campusWifi.vpnDisconnect"
          : "tools.campusWifi.vpnConnect",
      )}
    </UiButton>
  );

  return (
    <ToolCard
      icon={<ShieldCheck aria-hidden strokeWidth={1.75} />}
      title="tools.campusWifi.campusVpn"
      info={
        !unavailable && (
          <InfoTip label={t("tools.campusWifi.vpnRulesLabel")}>
            {t("tools.campusWifi.vpnRules")}
          </InfoTip>
        )
      }
      status={unavailable ? t("tools.campusWifi.vpnUnavailable") : vpnHint}
      note={unavailable ? undefined : "tools.campusWifi.vpnHint"}
      actions={
        !unavailable && (
          <>
            {vpn && (
              // The person's choice sits apart from the press, on the left.
              <div className="toolCardOption">
                <Switch
                  label={t("tools.campusWifi.vpnOnDemand")}
                  checked={Boolean(vpn.onDemand)}
                  disabled={vpnBusy}
                  onCheckedChange={(enabled) => void setOnDemand(enabled)}
                />
              </div>
            )}
            {action}
          </>
        )
      }
    />
  );
}
