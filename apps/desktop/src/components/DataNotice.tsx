import { Button, Modal } from "@mantine/core";
import { t } from "../i18n";

const ISSUES_URL = "https://github.com/isct-companion/Isct-Data/issues";

/** Shown once, before the user relies on collected school data. */
export function DataNotice({ acknowledge }: { acknowledge: () => void }) {
  return (
    <Modal
      opened
      onClose={acknowledge}
      closeOnClickOutside={false}
      title={t("Before you rely on this data")}
      centered
    >
      <div className="dataNotice">
        <p>
          {t(
            "IsctCompanion collects calendars, exam timetables and course details from Science Tokyo's official pages and documents, and we work to keep them accurate.",
          )}
        </p>
        <p>
          {t(
            "Much of this data is organised automatically, partly with AI, so it can contain mistakes. Please check anything important, especially exam dates, times and rooms, against the official source.",
          )}
        </p>
        <p>
          {t(
            "If something is wrong, right-click the session in Calendar to correct it, or edit it in Lectures. You can also report it on the data repository.",
          )}{" "}
          <a href={ISSUES_URL} target="_blank" rel="noreferrer">
            {t("Report a data problem")}
          </a>
        </p>
        <p>
          {t(
            "IsctCompanion is not responsible for any loss caused by errors in this data.",
          )}
        </p>
        <Button onClick={acknowledge}>{t("I understand")}</Button>
      </div>
    </Modal>
  );
}
