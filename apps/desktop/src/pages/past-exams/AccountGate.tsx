import { BookOpen, FileText } from "lucide-react";
import { Button, EmptyState, Skeleton } from "../../components/ui";
import { ExternalLink } from "../../components/ExternalLink";
import { desktopBridge } from "../../platform/native";
import { notify } from "../../components/notifications/notify";
import { t } from "../../i18n";
import type { ExamAccount, ExamSitePages } from "../../generated/persistence";

export type GateAction = "connect" | "acceptTerms" | "decline";

/** Everything before a usable session: sign in, the terms, or why not.
 * `pending` is the action running; while it runs no other starts. */
export function AccountGate({
  account,
  site,
  pending,
  run,
}: {
  account?: ExamAccount;
  site?: ExamSitePages;
  pending?: GateAction;
  run: (action: GateAction) => void;
}) {
  if (!account) return <Skeleton count={4} />;
  if (account.status === "unsupported")
    return (
      <EmptyState role="alert">{t("error.exam-share-unsupported")}</EmptyState>
    );
  if (account.status === "needsConsent")
    return (
      <EmptyState
        icon={FileText}
        action={
          <div className="examGateActions">
            {site && (
              <span>
                <ExternalLink href={site.terms}>
                  {t("examShare.terms")}
                </ExternalLink>
                {" · "}
                <ExternalLink href={site.privacy}>
                  {t("examShare.privacy")}
                </ExternalLink>
              </span>
            )}
            <Button
              loading={pending === "acceptTerms"}
              disabled={!!pending}
              onClick={() => run("acceptTerms")}
            >
              {t("examShare.accept")}
            </Button>
            <Button
              variant="subtle"
              loading={pending === "decline"}
              disabled={!!pending}
              onClick={() => run("decline")}
            >
              {t("examShare.decline")}
            </Button>
          </div>
        }
      >
        {t("examShare.consentHint")}
      </EmptyState>
    );
  // Only signing in shows here: a refusal that just ended is not one.
  const connecting = pending === "connect";
  return (
    <EmptyState
      icon={BookOpen}
      action={
        <div className="examGateActions">
          <Button loading={connecting} onClick={() => run("connect")}>
            {t("examShare.connect")}
          </Button>
          {connecting && (
            <Button
              variant="subtle"
              onClick={() =>
                void desktopBridge
                  ?.examShare?.({ kind: "cancelConnect" })
                  .catch((failure) => notify.error(failure))
              }
            >
              {t("common.cancel")}
            </Button>
          )}
        </div>
      }
    >
      {t(connecting ? "examShare.connectingHint" : "examShare.connectHint")}
    </EmptyState>
  );
}
