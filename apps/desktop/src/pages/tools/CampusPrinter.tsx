import { Printer } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  nativeOs,
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

/**
 * The campus printer: Core verifies the Tokyo Tech account and tries the
 * printer once each time the page opens and on Check, never in the
 * background. macOS and Linux add a system print queue; Windows adds a PDF
 * right-click entry and a system queue backed by the local IPP receiver.
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
    setChecking(false);
  }, []);

  useEffect(() => {
    void check();
    return () => {
      latest.current++;
    };
  }, [check]);

  const add = async (menu = windows) => {
    setAdding(true);
    setResult(undefined);
    try {
      const outcome = menu
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
      setAdding(false);
    }
  };

  const ready = status?.account.kind === "ready";
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
          {windows && (
            <span>{t("tools.campusPrinter.windowsSystemPrinter")}</span>
          )}
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
          <UiButton
            className="button small"
            disabled={!ready || adding || checking}
            title={t(
              windows
                ? "tools.campusPrinter.addsTheRightClickEntry"
                : "tools.campusPrinter.addsTheSystemPrinter",
            )}
            onClick={() => void add()}
          >
            {t(
              adding
                ? "tools.campusPrinter.adding"
                : windows
                  ? "tools.campusPrinter.addPdfMenu"
                  : "tools.campusPrinter.addSystemPrinter",
            )}
          </UiButton>
          {windows && (
            <UiButton
              className="button small"
              disabled={!ready || adding || checking}
              onClick={() => void add(false)}
            >
              {t(
                adding
                  ? "tools.campusPrinter.adding"
                  : "tools.campusPrinter.addSystemPrinter",
              )}
            </UiButton>
          )}
        </>
      }
    />
  );
}
