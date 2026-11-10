import { Printer } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  nativeOs,
  type CampusPrinterQueue,
  type CampusPrinterStatus,
  type PrinterAccount,
} from "../../platform/desktop-bridge";
import { accountText } from "../../features/campus-print/account";
import { UiButton } from "../../components/ui";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";

type Target = "menu" | "printer";

/** What the last Add said, kept as data so it follows a language switch. */
type Result =
  | { kind: "key"; key: MessageKey }
  | { kind: "account"; account: PrinterAccount }
  | { kind: "error"; text: string };

/** What an Add says: Windows names which of its two was added; macOS keeps
 * its own wording. */
const addedText = (target: Target, windows: boolean): MessageKey =>
  !windows
    ? "tools.campusPrinter.added"
    : target === "menu"
      ? "tools.campusPrinter.menuAdded"
      : "tools.campusPrinter.printerAdded";

/** Windows: the system printer's line on the card. */
const QUEUE_TEXT: Record<CampusPrinterQueue, MessageKey> = {
  ready: "tools.campusPrinter.systemPrinterReady",
  "not-installed": "tools.campusPrinter.systemPrinterMissing",
  "repair-required": "tools.campusPrinter.systemPrinterNeedsRepair",
  unavailable: "tools.campusPrinter.systemPrinterUnknown",
};

/** Windows: the system printer button names what it will do now. */
const QUEUE_ACTION: Record<CampusPrinterQueue, MessageKey> = {
  ready: "tools.campusPrinter.readdSystemPrinter",
  "not-installed": "tools.campusPrinter.addSystemPrinter",
  "repair-required": "tools.campusPrinter.repairSystemPrinter",
  unavailable: "tools.campusPrinter.repairSystemPrinter",
};

const resultText = (result: Result) =>
  result.kind === "key"
    ? t(result.key)
    : result.kind === "account"
      ? accountText(result.account)
      : result.text;

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
  const [adding, setAdding] = useState<Target>();
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

  const add = async (target: Target) => {
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
        setResult({ kind: "account", account: outcome.account });
      } else
        setResult({
          kind: "key",
          key:
            outcome.kind === "added"
              ? addedText(target, windows)
              : outcome.kind === "cancelled"
                ? "tools.campusPrinter.cancelled"
                : "tools.campusPrinter.couldNotAdd",
        });
    } catch (error) {
      const message = errorText(error) ?? t("tools.campusPrinter.couldNotAdd");
      setResult({ kind: "error", text: message });
      notify(message);
    } finally {
      setAdding(undefined);
      if (target === "printer" && windows) void check();
    }
  };

  const ready = status?.account.kind === "ready";
  const busy = !ready || adding !== undefined || checking;
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
          {queue && <span>{t(QUEUE_TEXT[queue])}</span>}
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
          {windows && (
            <UiButton
              className="button small"
              disabled={busy}
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
            disabled={busy}
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
                : QUEUE_ACTION[queue ?? "not-installed"],
            )}
          </UiButton>
        </>
      }
    />
  );
}
