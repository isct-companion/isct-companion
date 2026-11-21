import { t, type MessageKey } from "../i18n";
import type { VpnNotice, VpnStatus } from "../platform/desktop-bridge";

/** Codes from the desktop's VPN commands, keyed as the shell reports them. */
const VPN_ERROR_TEXT: Record<string, MessageKey> = {
  helper_not_installed: "tools.campusWifi.vpnHelperNotInstalled",
  helper_needs_approval: "tools.campusWifi.vpnHelperNeedsApproval",
  helper_outdated: "tools.campusWifi.vpnHelperOutdated",
  helper_not_starting: "tools.campusWifi.vpnHelperNotStarting",
  helper_elsewhere: "tools.campusWifi.vpnHelperElsewhere",
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

export const vpnErrorText = (status: VpnStatus) =>
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
