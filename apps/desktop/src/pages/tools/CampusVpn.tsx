import { Clock, Route, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import type { VpnNotice, VpnStatus } from "../../platform/desktop-bridge";
import { InfoTip, Switch, UiButton } from "../../components/ui";
import { VpnPublishers } from "./VpnPublishers";
import { workspaceActions } from "../../store/workspace";
import { confirmAction } from "../../components/confirm";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";
import { jstClock } from "../../lib/civil-date";

/** Codes from the desktop's VPN commands, keyed as the shell reports them. */
const VPN_ERROR_TEXT: Record<string, MessageKey> = {
  helper_not_installed: "tools.campusWifi.vpnHelperNotInstalled",
  helper_needs_approval: "tools.campusWifi.vpnHelperNeedsApproval",
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

/** This device's publisher list; `undefined` stores the default. */
export const saveVpnPublishers = (list: string[] | undefined) =>
  workspaceActions.updateSettings((draft) => {
    draft.settings.campusVpnPublishers = list;
    return draft;
  });

/** The APM session's countdown as h:mm, or nothing when it is not known. */
const vpnRemaining = (expiresAt: string | null | undefined, now: number) => {
  const end = expiresAt ? Date.parse(expiresAt) : NaN;
  if (Number.isNaN(end)) return undefined;
  const minutes = Math.max(0, Math.floor((end - now) / 60_000));
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
};

type Badge = { key: MessageKey; role?: "ok" | "accent" | "warn" | "danger" };

/** The one word for the state, as the card's badge says it. */
const vpnBadge = (vpn: VpnStatus | undefined, busy: boolean): Badge =>
  vpn?.error
    ? { key: "tools.campusWifi.vpnBadgeError", role: "danger" }
    : vpn?.state === "connected"
      ? { key: "tools.campusWifi.vpnBadgeConnected", role: "ok" }
      : busy || vpn?.state === "connecting"
        ? { key: "tools.campusWifi.vpnBadgeConnecting", role: "accent" }
        : vpn?.onCampus
          ? { key: "tools.campusWifi.vpnBadgeOnCampus" }
          : vpn?.demandPausedUntil
            ? { key: "tools.campusWifi.vpnBadgePaused", role: "warn" }
            : vpn?.onDemand
              ? { key: "tools.campusWifi.vpnBadgeStandby" }
              : { key: "tools.campusWifi.vpnBadgeOff" };

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

  // A helper still waiting for the person to allow it (macOS Login Items)
  // is shown as such and goes no further: a connect would spend a one-time
  // login on a helper that cannot start the tunnel.
  const allowed = (next: VpnStatus) => {
    if (next.error !== "helper_needs_approval") return next;
    setVpn(next);
    return undefined;
  };

  // The helper, installed or updated once with the person's consent; the
  // status after it, or nothing when they declined or it waits for approval.
  const readyHelper = async () => {
    if (!desktopBridge?.vpnStatus) return;
    const next = await desktopBridge.vpnStatus();
    // Allowed by now, or System Settings opens at it once more.
    if (next.error === "helper_needs_approval")
      return allowed(await desktopBridge.vpnInstall!());
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
    return confirmed ? allowed(await desktopBridge.vpnInstall!()) : undefined;
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

  // Persist first, then let Core take it and route it; Core normalizes, so
  // an entry it could not read is what the returned list leaves out.
  const setPublishers = async (list: string[] | undefined) => {
    if (!desktopBridge?.vpnPublishers) return;
    const next = await desktopBridge.vpnPublishers(list ?? null);
    setVpn(next);
    await saveVpnPublishers(
      next.publishersDefault ? undefined : next.publishers,
    );
    return next;
  };

  const unavailable = vpn?.state === "unavailable";
  const badge = vpnBadge(vpn, vpnBusy);
  const remaining = vpnConnected ? vpnRemaining(vpn.expiresAt, now) : undefined;
  // A helper that does not answer is installed again: its message says so,
  // and installing re-registers the daemon from this app.
  const helperNeeded =
    vpn?.error === "helper_unreachable" ||
    vpn?.error === "helper_not_installed" ||
    vpn?.error === "helper_outdated" ||
    vpn?.error === "helper_needs_approval";
  // A line only when something needs saying beyond the badge.
  const detail = vpn?.error
    ? vpnErrorText(vpn)
    : vpn?.demandPausedUntil && !vpnConnected
      ? t("tools.campusWifi.vpnPausedUntil", {
          time: jstClock(new Date(vpn.demandPausedUntil)),
        })
      : undefined;
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
          : vpn?.error === "helper_needs_approval"
            ? "tools.campusWifi.vpnAllow"
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

  if (unavailable)
    return (
      <ToolCard
        icon={<ShieldCheck aria-hidden strokeWidth={1.75} />}
        title="tools.campusWifi.campusVpn"
        status={t("tools.campusWifi.vpnUnavailable")}
        actions={null}
      />
    );
  return (
    <ToolCard
      icon={<ShieldCheck aria-hidden strokeWidth={1.75} />}
      title="tools.campusWifi.campusVpn"
      info={
        <InfoTip label={t("tools.campusWifi.vpnRulesLabel")}>
          {t("tools.campusWifi.vpnRules")}
        </InfoTip>
      }
      badge={
        vpn && (
          <span className="uiTag" data-role={badge.role}>
            {t(badge.key)}
          </span>
        )
      }
      actions={
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
      }
    >
      {vpn && (
        <dl className="toolCardFacts">
          <div>
            <dt>
              <Route aria-hidden size={14} strokeWidth={1.75} />
            </dt>
            <dd>
              {t("tools.campusWifi.vpnRoutes", {
                count: vpn.publishers?.length ?? 0,
              })}
              <VpnPublishers
                publishers={vpn.publishers ?? []}
                isDefault={vpn.publishersDefault ?? true}
                onChange={setPublishers}
              />
            </dd>
          </div>
          {remaining && (
            <div>
              <dt>
                <Clock aria-hidden size={14} strokeWidth={1.75} />
              </dt>
              <dd>
                {t("tools.campusWifi.vpnSessionLeft", { time: remaining })}
              </dd>
            </div>
          )}
          {detail && <p className="toolCardDetail">{detail}</p>}
        </dl>
      )}
    </ToolCard>
  );
}
