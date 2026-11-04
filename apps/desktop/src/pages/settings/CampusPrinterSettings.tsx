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
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { errorText } from "../../app/shared";

const SETUP_TEXT: Record<
  Exclude<CampusPrinterSetup["kind"], "account">,
  MessageKey
> = {
  added: "settings.campusPrinter.added",
  cancelled: "settings.campusPrinter.cancelled",
  failed: "settings.campusPrinter.couldNotAdd",
};

/**
 * The campus printer: Core verifies the Tokyo Tech account and tries the
 * printer once each time the page opens and on Check, never in the
 * background. macOS and Linux add a system print queue; Windows adds a PDF
 * right-click entry.
 */
export function CampusPrinterSettings({
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

  const add = async () => {
    setAdding(true);
    setResult(undefined);
    try {
      const outcome = windows
        ? await desktopBridge?.addCampusPrintMenu?.(
            t("settings.campusPrinter.printToCampusPrinter"),
          )
        : await desktopBridge?.installCampusPrinter?.(
            t("settings.campusPrinter.adminPrompt"),
          );
      if (!outcome) return;
      if (outcome.kind === "account") {
        setStatus((current) =>
          current ? { ...current, account: outcome.account } : current,
        );
        setResult(accountText(outcome.account));
      } else setResult(t(SETUP_TEXT[outcome.kind]));
    } catch (error) {
      const message =
        errorText(error) ?? t("settings.campusPrinter.couldNotAdd");
      setResult(message);
      notify(message);
    } finally {
      setAdding(false);
    }
  };

  const ready = status?.account.kind === "ready";
  return (
    <>
      <SettingsHeader
        title="settings.campusPrinter.campusPrinter"
        description="settings.campusPrinter.printsToTheKyoceraPrinter"
      />
      <SettingsGroup footer={t("settings.campusPrinter.checksOnlyWhenYouOpen")}>
        <Row
          group
          id="campus-printer"
          title="settings.campusPrinter.printer"
          hint={
            checking
              ? t("settings.academicCalendar.checking")
              : status?.reach === "reachable"
                ? t("settings.campusPrinter.readyToPrint")
                : t("settings.campusPrinter.cannotReachThePrinter")
          }
        >
          <UiButton
            className="button small"
            disabled={checking}
            onClick={() => void check()}
          >
            {t("settings.campusPrinter.check")}
          </UiButton>
        </Row>
        <Row
          id="student-id"
          title="settings.campusPrinter.studentId"
          hint={
            status
              ? accountText(status.account)
              : checking
                ? t("settings.academicCalendar.checking")
                : undefined
          }
        >
          {null}
        </Row>
      </SettingsGroup>
      <SettingsGroup
        footer={t(
          windows
            ? "settings.campusPrinter.addsTheRightClickEntry"
            : "settings.campusPrinter.addsTheSystemPrinter",
        )}
      >
        <Row
          group
          id="campus-printer-add"
          title={
            windows
              ? "settings.campusPrinter.pdfRightClickMenu"
              : "settings.campusPrinter.systemPrinter"
          }
          hint={result}
        >
          <UiButton
            className="button small"
            disabled={!ready || adding || checking}
            onClick={() => void add()}
          >
            {t(
              adding
                ? "settings.campusPrinter.adding"
                : "settings.campusPrinter.add",
            )}
          </UiButton>
        </Row>
      </SettingsGroup>
    </>
  );
}
