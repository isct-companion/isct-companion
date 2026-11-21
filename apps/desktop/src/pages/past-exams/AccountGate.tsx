import { BookOpen, FileText } from "lucide-react";
import { Button, EmptyState, Skeleton } from "../../components/ui";
import { ExternalLink } from "../../components/ExternalLink";
import { desktopBridge } from "../../platform/native";
import { notify } from "../../components/notifications/notify";
import { t } from "../../i18n";
import type { ExamAccount } from "../../generated/persistence";

/** Everything before a usable session: sign in, the terms, or why not. */
export function AccountGate({
  account,
  pending,
  connect,
  accept,
}: {
  account?: ExamAccount;
  pending: boolean;
  connect: () => void;
  accept: () => void;
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
            <span>
              <ExternalLink href="https://sciencetokyoexam.com/terms">
                {t("examShare.terms")}
              </ExternalLink>
              {" · "}
              <ExternalLink href="https://sciencetokyoexam.com/privacy">
                {t("examShare.privacy")}
              </ExternalLink>
            </span>
            <Button loading={pending} onClick={accept}>
              {t("examShare.accept")}
            </Button>
          </div>
        }
      >
        {t("examShare.consentHint")}
      </EmptyState>
    );
  return (
    <EmptyState
      icon={BookOpen}
      action={
        <div className="examGateActions">
          <Button loading={pending} onClick={connect}>
            {t("examShare.connect")}
          </Button>
          {pending && (
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
      {t(pending ? "examShare.connectingHint" : "examShare.connectHint")}
    </EmptyState>
  );
}
