import { useEffect, useState } from "react";
import { t } from "../../i18n";
import { UiButton } from "../../components/ui";
import {
  chooseStorage,
  storageInfo,
  type StorageInfo,
} from "../../features/lms/files";
export function CourseStorage() {
  const [info, setInfo] = useState<StorageInfo>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void storageInfo()
      .then(setInfo)
      .catch((e) => setError(String(e)));
  }, []);
  const choose = async (reset: boolean) => {
    setBusy(true);
    setError("");
    try {
      setInfo(await chooseStorage(reset));
      dispatchEvent(new Event("course-storage-changed"));
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="courseStorageSettings" data-setting="folder">
      <b>{t("settings.courseStorage.courseFolder")}</b>
      <p>{info?.root && info.root}</p>
      <small>{t("settings.courseStorage.changingItOnlyAffectsNew")}</small>
      <div className="actions">
        {info?.canChoose && (
          <UiButton
            className="button small"
            disabled={busy}
            onClick={() => void choose(false)}
          >
            {t("settings.courseStorage.chooseFolder")}
          </UiButton>
        )}
        {info?.custom && (
          <UiButton
            className="textButton"
            disabled={busy}
            onClick={() => void choose(true)}
          >
            {t("settings.courseStorage.useDocuments")}
          </UiButton>
        )}
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
