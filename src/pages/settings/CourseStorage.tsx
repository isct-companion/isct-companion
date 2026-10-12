import { useEffect, useState } from "react";
import { t } from "../../i18n";
import {
  chooseStorage,
  storageInfo,
  type StorageInfo,
} from "../today/lms/files";
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
      <b>{t("Course folder")}</b>
      <p>{info?.root}</p>
      <small>{t("Changing it only affects new downloads.")}</small>
      <div className="actions">
        {info?.canChoose && (
          <button
            type="button"
            className="button small"
            disabled={busy}
            onClick={() => void choose(false)}
          >
            {t("Choose folder…")}
          </button>
        )}
        {info?.custom && (
          <button
            type="button"
            className="textButton"
            disabled={busy}
            onClick={() => void choose(true)}
          >
            {t("Use Documents")}
          </button>
        )}
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
