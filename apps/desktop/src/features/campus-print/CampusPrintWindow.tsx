import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n, { t } from "../../i18n";
import {
  getDesktopBridge,
  type CampusPrintOptions,
  type CampusPrintOutcome,
  type CampusPrintRequest,
} from "../../platform/desktop-bridge";
import { NumberInput, UiButton } from "../../components/ui";
import { Choice, Row, SettingsGroup } from "../../components/forms";
import { accountText } from "./account";
import { errorText } from "../../app/shared";
import "../../pages/settings/settings.css";
import "./campus-print.css";

const bridge = getDesktopBridge();
const MAX_COPIES = 99;

function outcomeText(outcome: CampusPrintOutcome) {
  switch (outcome.kind) {
    case "printed":
      return t("campusPrint.window.sent", { copies: outcome.copies });
    case "account":
      return accountText(outcome.account);
    case "not-printable":
      return t("campusPrint.window.notAPdf");
    case "invalid-copies":
      return t("campusPrint.window.copiesRange");
    case "unreadable":
      return t("campusPrint.window.couldNotReadTheFile");
    case "unreachable":
      return outcome.printed > 0
        ? t("campusPrint.window.unreachableAfter", { printed: outcome.printed })
        : t("tools.campusPrinter.cannotReachThePrinter");
    case "not-printed":
      return outcome.printed > 0
        ? t("campusPrint.window.notPrintedAfter", { printed: outcome.printed })
        : t("campusPrint.window.notPrinted");
    case "uncertain":
      return outcome.printed > 0
        ? t("campusPrint.window.uncertainAfter", { printed: outcome.printed })
        : t("campusPrint.window.uncertain");
  }
}

/** Applies the device's language and look, which the main window reads
 * from the workspace this window does not load. */
function adopt(request: CampusPrintRequest) {
  const locale = request.locale ?? "en";
  void i18n.changeLanguage(locale);
  document.documentElement.lang = locale;
  if (request.appearance)
    document.documentElement.dataset.appearance = request.appearance;
  document.documentElement.dataset.theme = request.theme ?? "warm";
}

/**
 * The Windows print window, opened from File Explorer's PDF right-click
 * entry: copies, sides and color, then one send. Core bills the verified
 * student ID; this window never names one.
 */
export function CampusPrintWindow() {
  const [request, setRequest] = useState<CampusPrintRequest>();
  const [copies, setCopies] = useState<number | string>(1);
  const [duplex, setDuplex] =
    useState<NonNullable<CampusPrintOptions["duplex"]>>("long-edge");
  const [color, setColor] =
    useState<NonNullable<CampusPrintOptions["color"]>>("mono");
  const [sending, setSending] = useState(false);
  const [result, setResult] = useState<string>();
  const [sent, setSent] = useState(false);

  useEffect(() => {
    void bridge
      ?.campusPrintRequest?.()
      .then((next) => {
        adopt(next);
        document.title = t("campusPrint.window.printToCampusPrinter");
        setRequest(next);
      })
      .catch((error: unknown) => setResult(errorText(error)));
  }, []);

  const count = typeof copies === "number" ? copies : NaN;
  const valid = Number.isInteger(count) && count >= 1 && count <= MAX_COPIES;
  const close = () => void getCurrentWindow().close();

  const print = async () => {
    if (!bridge?.campusPrint || !valid) return;
    setSending(true);
    setResult(undefined);
    try {
      const outcome = await bridge.campusPrint({
        copies: count,
        duplex,
        color,
      });
      setResult(outcomeText(outcome));
      setSent(outcome.kind === "printed");
    } catch (error) {
      setResult(errorText(error));
    } finally {
      setSending(false);
    }
  };

  return (
    <main className="campusPrint settings">
      <h1>{t("campusPrint.window.printToCampusPrinter")}</h1>
      <p className="campusPrintFile">{request?.fileName ?? "…"}</p>
      <SettingsGroup>
        <Row group title="campusPrint.window.copies">
          <NumberInput
            className="campusPrintCopies"
            min={1}
            max={MAX_COPIES}
            value={copies}
            disabled={sending || sent}
            onChange={setCopies}
            aria-label={t("campusPrint.window.copies")}
          />
        </Row>
        <Row group title="campusPrint.window.sides">
          <Choice
            value={duplex}
            disabled={sending || sent}
            onChange={setDuplex}
            options={[
              { value: "off", label: t("campusPrint.window.oneSided") },
              { value: "long-edge", label: t("campusPrint.window.longEdge") },
              { value: "short-edge", label: t("campusPrint.window.shortEdge") },
            ]}
          />
        </Row>
        <Row group title="campusPrint.window.color">
          <Choice
            value={color}
            disabled={sending || sent}
            onChange={setColor}
            options={[
              { value: "mono", label: t("campusPrint.window.blackAndWhite") },
              { value: "color", label: t("campusPrint.window.inColor") },
            ]}
          />
        </Row>
      </SettingsGroup>
      <p className="campusPrintResult" role="status">
        {!valid && copies !== "" ? t("campusPrint.window.copiesRange") : result}
      </p>
      <footer>
        {/* Closing never cancels a send: it runs on to its end in the
            background, bounded by the LPD timeouts, and logs its result. */}
        <UiButton className="button" onClick={close}>
          {t(sent ? "common.done" : sending ? "common.close" : "common.cancel")}
        </UiButton>
        {!sent && (
          <UiButton
            className="button primary"
            disabled={!request || !valid || sending}
            onClick={() => void print()}
          >
            {t(
              sending
                ? "campusPrint.window.sending"
                : "campusPrint.window.print",
            )}
          </UiButton>
        )}
      </footer>
    </main>
  );
}
