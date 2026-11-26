import { notifications } from "./notifications";
import { t } from "../../i18n";
import { errorText } from "../../lib/error-text";

type Action = { label: string; run: () => void };
const show = (message: string, action?: Action) =>
  notifications.show({ id: "workspace-notice", message, action });

let undoOperation = 0;

export const notify = {
  info: show,
  undoable: (message: string, undo: () => void) =>
    notifications.show({
      id: `undo-${++undoOperation}`,
      message,
      undo: true,
      action: { label: t("common.undo"), run: undo },
    }),
  error: (error: unknown, retry?: () => void) => {
    const message = errorText(error) ?? t("common.unknownError");
    show(message, retry && { label: t("app.retrySaving"), run: retry });
  },
};
