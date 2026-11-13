import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import i18n, { t } from "../../i18n";
import {
  getDesktopBridge,
  type CampusPrintRequest,
  type CampusPrintSettings,
} from "../../platform/desktop-bridge";
import { NumberInput, UiButton } from "../../components/ui";
import { Choice, Row, SettingsGroup } from "../../components/forms";
import { errorText } from "../../app/shared";
import "../../pages/settings/settings.css";
import "./campus-print.css";

const bridge = getDesktopBridge();

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
  /** Core's defaults until the person picks; unset until the request. */
  const [settings, setSettings] = useState<CampusPrintSettings>();
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
        setSettings(next.defaults);
      })
      .catch((error: unknown) => setResult(errorText(error)));
  }, []);

  const count = typeof copies === "number" ? copies : NaN;
  const valid =
    request !== undefined &&
    Number.isInteger(count) &&
    count >= 1 &&
    count <= request.maxCopies;
  const close = () => void getCurrentWindow().close();

  const print = async () => {
    if (!bridge?.campusPrint || !valid || !settings) return;
    setSending(true);
    setResult(undefined);
    try {
      const report = await bridge.campusPrint({ copies: count, ...settings });
      setResult(t(report.message.key, report.message.args));
      setSent(report.printed);
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
      {settings && (
        <SettingsGroup>
          <Row group title="campusPrint.window.copies">
            <NumberInput
              className="campusPrintCopies"
              min={1}
              max={request?.maxCopies}
              value={copies}
              disabled={sending || sent}
              onChange={setCopies}
              aria-label={t("campusPrint.window.copies")}
            />
          </Row>
          <Row group title="campusPrint.window.sides">
            <Choice
              value={settings.sides}
              disabled={sending || sent}
              onChange={(sides) => setSettings({ ...settings, sides })}
              options={[
                { value: "one-sided", label: t("campusPrint.window.oneSided") },
                { value: "long-edge", label: t("campusPrint.window.longEdge") },
                { value: "short-edge", label: t("campusPrint.window.shortEdge") },
              ]}
            />
          </Row>
          <Row group title="campusPrint.window.color">
            <Choice
              value={settings.color}
              disabled={sending || sent}
              onChange={(color) => setSettings({ ...settings, color })}
              options={[
                { value: "mono", label: t("campusPrint.window.blackAndWhite") },
                { value: "color", label: t("campusPrint.window.inColor") },
              ]}
            />
          </Row>
        </SettingsGroup>
      )}
      <p className="campusPrintResult" role="status">
        {request && !valid && copies !== ""
          ? t("campusPrint.window.copiesRange", { max: request.maxCopies })
          : result}
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
            disabled={!settings || !valid || sending}
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
