import { Printer } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  nativeOs,
  type CampusPrinterStatus,
  type PrinterAccount,
} from "../../platform/desktop-bridge";
import { accountText } from "../../features/campus-print/account";
import { UiButton } from "../../components/ui";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";

/** What the last Add said, kept as data so it follows a language switch. */
type Result =
  | { kind: "key"; key: MessageKey }
  | { kind: "account"; account: PrinterAccount }
  | { kind: "error"; text: string };

const resultText = (result: Result) =>
  result.kind === "key"
    ? t(result.key)
    : result.kind === "account"
      ? accountText(result.account)
      : result.text;

/**
 * The campus printer: Core verifies the Tokyo Tech account and tries the
 * printer once each time the page opens and on Check, never in the
 * background. The system print dialog sends jobs through the queue.
 */
export function CampusPrinter({
  notify,
}: {
  notify: (message: string) => void;
}) {
  const windows = nativeOs() === "windows";
  const [status, setStatus] = useState<CampusPrinterStatus>();
  const [checking, setChecking] = useState(true);
  const [adding, setAdding] = useState(false);
  const [result, setResult] = useState<Result>();
  const latest = useRef(0);

  const check = useCallback(async () => {
    const call = ++latest.current;
    setChecking(true);
    const next = await desktopBridge
      ?.campusPrinterStatus?.()
      .catch(() => undefined);
    if (call !== latest.current) return;
    setStatus(next);
    setChecking(false);
  }, []);

  useEffect(() => {
    void check();
    return () => {
      latest.current++;
    };
  }, [check]);

  const add = async () => {
    setAdding(true);
    setResult(undefined);
    try {
      const outcome = await desktopBridge?.installCampusPrinter?.(
        t("tools.campusPrinter.adminPrompt"),
      );
      if (!outcome) return;
      if (outcome.kind === "account") {
        setStatus((current) =>
          current ? { ...current, account: outcome.account } : current,
        );
        setResult({ kind: "account", account: outcome.account });
      } else
        setResult({
          kind: "key",
          key:
            outcome.kind === "added"
              ? windows
                ? "tools.campusPrinter.printerAdded"
                : "tools.campusPrinter.added"
              : outcome.kind === "cancelled"
                ? "tools.campusPrinter.cancelled"
                : "tools.campusPrinter.couldNotAdd",
        });
    } catch (error) {
      const message = errorText(error) ?? t("tools.campusPrinter.couldNotAdd");
      setResult({ kind: "error", text: message });
      notify(message);
    } finally {
      setAdding(false);
    }
  };

  const ready = status?.account.kind === "ready";
  const busy = !ready || adding || checking;
  return (
    <ToolCard
      icon={<Printer aria-hidden strokeWidth={1.75} />}
      title="tools.campusPrinter.campusPrinter"
      status={
        <>
          <span>
            {checking
              ? t("settings.academicCalendar.checking")
              : status?.reach === "reachable"
                ? t("tools.campusPrinter.readyToPrint")
                : t("tools.campusPrinter.cannotReachThePrinter")}
          </span>
          {status && <span>{accountText(status.account)}</span>}
          {result && <span>{resultText(result)}</span>}
        </>
      }
      note="tools.campusPrinter.printsToTheKyoceraPrinter"
      actions={
        <>
          <UiButton
            className="button small"
            disabled={checking}
            onClick={() => void check()}
          >
            {t("tools.campusPrinter.check")}
          </UiButton>
          <UiButton
            className="button small"
            disabled={busy}
            aria-busy={adding || undefined}
            title={t(
              windows
                ? "tools.campusPrinter.addsTheWindowsPrinter"
                : "tools.campusPrinter.addsTheSystemPrinter",
            )}
            onClick={() => void add()}
          >
            {t("tools.campusPrinter.addSystemPrinter")}
          </UiButton>
        </>
      }
    />
  );
}
