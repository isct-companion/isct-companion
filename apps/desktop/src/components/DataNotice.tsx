import type { NoticeDialog } from "./confirm";
import { t } from "../i18n";

const ISSUES_URL = "https://github.com/isct-companion/Isct-Data/issues";

/** Shown once, before the user relies on collected school data. */
export function dataNoticeDialog(
  attempt: number,
  acknowledge: () => void,
): NoticeDialog {
  return {
    id: `data-notice:${attempt}`,
    title: t("dataNotice.beforeYouRelyOnThis"),
    closeOnClickOutside: false,
    message: (
      <div className="dataNotice">
        <p>{t("dataNotice.isctcompanionCollectsCalendarsExamTimetables")}</p>
        <p>{t("dataNotice.muchOfThisDataIs")}</p>
        <p>
          {t("dataNotice.ifSomethingIsWrongRight")}{" "}
          <a href={ISSUES_URL} target="_blank" rel="noreferrer">
            {t("dataNotice.reportADataProblem")}
          </a>
        </p>
        <p>{t("dataNotice.isctcompanionIsNotResponsibleFor")}</p>
      </div>
    ),
    choices: [{ id: "acknowledge", label: t("dataNotice.iUnderstand") }],
    onChoice: () => acknowledge(),
  };
}
