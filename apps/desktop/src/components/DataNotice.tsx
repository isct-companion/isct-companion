import type { NoticeDialog } from "./confirm";
import { DataNoticeLine } from "./guide";
import { t } from "../i18n";

/** Shown once, before the user relies on collected school data, on a device
 * that has already seen the welcome sheet (which otherwise carries it). */
export function dataNoticeDialog(
  attempt: number,
  acknowledge: () => void,
): NoticeDialog {
  return {
    id: `data-notice:${attempt}`,
    title: t("dataNotice.beforeYouRelyOnThis"),
    closeOnClickOutside: false,
    message: <DataNoticeLine />,
    choices: [{ id: "acknowledge", label: t("dataNotice.iUnderstand") }],
    onChoice: () => acknowledge(),
  };
}
