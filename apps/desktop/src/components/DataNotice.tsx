import { Button, Modal } from "./ui";
import { t } from "../i18n";

const ISSUES_URL = "https://github.com/isct-companion/Isct-Data/issues";

/** Shown once, before the user relies on collected school data. */
export function DataNotice({ acknowledge }: { acknowledge: () => void }) {
  return (
    <Modal
      opened
      onClose={acknowledge}
      closeOnClickOutside={false}
      title={t("dataNotice.beforeYouRelyOnThis")}
      centered
    >
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
        <Button onClick={acknowledge}>{t("dataNotice.iUnderstand")}</Button>
      </div>
    </Modal>
  );
}
