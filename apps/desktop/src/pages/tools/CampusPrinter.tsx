import { Printer } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  nativeOs,
  type CampusPrinterQueue,
  type CampusPrinterSetup,
  type CampusPrinterStatus,
} from "../../platform/desktop-bridge";
import { accountText } from "../../features/campus-print/account";
import { UiButton } from "../../components/ui";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";

const SETUP_TEXT: Record<
  Exclude<CampusPrinterSetup["kind"], "account">,
  MessageKey
> = {
  added: "tools.campusPrinter.added",
  cancelled: "tools.campusPrinter.cancelled",
  failed: "tools.campusPrinter.couldNotAdd",
};

/** Windows: the system printer's line on the card; nothing when unknown. */
const QUEUE_TEXT: Partial<Record<CampusPrinterQueue, MessageKey>> = {
  ready: "tools.campusPrinter.systemPrinterReady",
  "not-installed": "tools.campusPrinter.systemPrinterMissing",
  "repair-required": "tools.campusPrinter.systemPrinterNeedsRepair",
};

/**
 * The campus printer: Core verifies the Tokyo Tech account and tries the
 * printer once each time the page opens and on Check, never in the
 * background. macOS adds a system print queue; Windows adds a PDF
 * right-click entry and a system printer served while the app runs.
 */
export function CampusPrinter({
  notify,
}: {
  notify: (message: string) => void;
}) {
  const windows = nativeOs() === "windows";
  const [status, setStatus] = useState<CampusPrinterStatus>();
  const [checking, setChecking] = useState(true);
  const [queue, setQueue] = useState<CampusPrinterQueue>();
  const [adding, setAdding] = useState<"menu" | "printer">();
  const [result, setResult] = useState<string>();
  const latest = useRef(0);

  const check = useCallback(async () => {
    const call = ++latest.current;
    setChecking(true);
    const next = await desktopBridge
      ?.campusPrinterStatus?.()
      .catch(() => undefined);
    if (call !== latest.current) return;
    setStatus(next);
    if (windows) {
      const installed = await desktopBridge
        ?.campusPrinterQueue?.()
        .catch(() => undefined);
      if (call !== latest.current) return;
      setQueue(installed);
    }
    setChecking(false);
  }, [windows]);

  useEffect(() => {
    void check();
    return () => {
      latest.current++;
    };
  }, [check]);

  const add = async (target: "menu" | "printer") => {
    setAdding(target);
    setResult(undefined);
    try {
      const outcome =
        target === "menu"
          ? await desktopBridge?.addCampusPrintMenu?.(
              t("tools.campusPrinter.printToCampusPrinter"),
            )
          : await desktopBridge?.installCampusPrinter?.(
              t("tools.campusPrinter.adminPrompt"),
            );
      if (!outcome) return;
      if (outcome.kind === "account") {
        setStatus((current) =>
          current ? { ...current, account: outcome.account } : current,
        );
        setResult(accountText(outcome.account));
      } else setResult(t(SETUP_TEXT[outcome.kind]));
    } catch (error) {
      const message = errorText(error) ?? t("tools.campusPrinter.couldNotAdd");
      setResult(message);
      notify(message);
    } finally {
      setAdding(undefined);
      if (target === "printer" && windows) void check();
    }
  };

  const ready = status?.account.kind === "ready";
  const queueText = queue && QUEUE_TEXT[queue];
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
          {queueText && <span>{t(queueText)}</span>}
          {result && <span>{result}</span>}
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
          {windows && (
            <UiButton
              className="button small"
              disabled={!ready || adding !== undefined || checking}
              title={t("tools.campusPrinter.addsTheRightClickEntry")}
              onClick={() => void add("menu")}
            >
              {t(
                adding === "menu"
                  ? "tools.campusPrinter.adding"
                  : "tools.campusPrinter.addPdfMenu",
              )}
            </UiButton>
          )}
          <UiButton
            className="button small"
            disabled={!ready || adding !== undefined || checking}
            title={t(
              windows
                ? "tools.campusPrinter.addsTheWindowsPrinter"
                : "tools.campusPrinter.addsTheSystemPrinter",
            )}
            onClick={() => void add("printer")}
          >
            {t(
              adding === "printer"
                ? "tools.campusPrinter.adding"
                : queue === "repair-required"
                  ? "tools.campusPrinter.repairSystemPrinter"
                  : "tools.campusPrinter.addSystemPrinter",
            )}
          </UiButton>
        </>
      }
    />
  );
}
