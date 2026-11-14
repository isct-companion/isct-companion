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
import { InfoTip, UiButton } from "../../components/ui";
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
  const reachable = status?.reach === "reachable";
  // One line: what stops printing first, else that it can print and whose
  // ID it bills. An Add outcome takes its place until the next check.
  const line = checking
    ? t("settings.academicCalendar.checking")
    : result
      ? resultText(result)
      : !status
        ? t("tools.campusPrinter.connectScienceTokyo")
        : !ready
          ? accountText(status.account)
          : !reachable
            ? t("tools.campusPrinter.connectScienceTokyo")
            : `${t("tools.campusPrinter.readyToPrint")} · ${accountText(status.account)}`;
  return (
    <ToolCard
      icon={<Printer aria-hidden strokeWidth={1.75} />}
      title="tools.campusPrinter.campusPrinter"
      info={
        <InfoTip label={t("tools.campusPrinter.printerRulesLabel")}>
          {t(
            windows
              ? "tools.campusPrinter.printerRulesWindows"
              : "tools.campusPrinter.printerRulesMac",
          )}
        </InfoTip>
      }
      status={line}
      note="tools.campusPrinter.printerHint"
      actions={
        <>
          {/* The page checks on opening; again is only for after a change. */}
          {!checking && (!ready || !reachable) && (
            <UiButton className="button small" onClick={() => void check()}>
              {t("tools.campusPrinter.checkAgain")}
            </UiButton>
          )}
          {ready && (
            <UiButton
              className="button small"
              disabled={adding || checking}
              aria-busy={adding || undefined}
              onClick={() => void add()}
            >
              {t("tools.campusPrinter.addSystemPrinter")}
            </UiButton>
          )}
        </>
      }
    />
  );
}
