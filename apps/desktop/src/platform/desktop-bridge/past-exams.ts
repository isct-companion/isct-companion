import type {
  ExamAccount,
  ExamAction,
  ExamAnswer,
  ExamUploadFile,
} from "../../generated/persistence";

import {
  subscribe,
  type BridgeCommands,
  type ListenNative,
  type SavedDownload,
} from "./helpers";

export interface PastExamsBridge {
  examShare?(action: ExamAction): Promise<ExamAnswer>;
  /** Every `exam-account` event: the account and its upload progress. */
  onExamAccount?(callback: (account: ExamAccount) => void): () => void;
  examUploadPickFiles?(title: string): Promise<ExamUploadFile[] | null>;
  examUploadClearFiles?(): Promise<void>;
  /** Opens a Core past-exam file in the system default application. */
  examOpenFile?(path: string): Promise<void>;
  /** Keeps the opened version for offline use, returning whether the library changed. */
  examRetainFile?(id: string, path: string): Promise<boolean>;
  /** Copies the opened past-exam version into Downloads under the name Core
   * gives it (the uploaded name, suffixed to match its validated type). */
  examSaveCopy?(id: string, path: string): Promise<SavedDownload>;
}

export function pastExamsCommands(
  { result, invokeCommand, booleanCommand }: BridgeCommands,
  listen: ListenNative,
): PastExamsBridge {
  return {
    examShare: (action) => result("exam_share", { action }),
    onExamAccount: (callback) =>
      subscribe<ExamAccount>(
        listen,
        "exam-account",
        (payload) =>
          payload && typeof payload.status === "string" ? payload : undefined,
        callback,
      ),
    examUploadPickFiles: async (title) => {
      const command = "exam_upload_pick_files";
      const files = await invokeCommand<ExamUploadFile[] | null>(command, {
        title,
      });
      if (files === null) return null;
      if (
        !Array.isArray(files) ||
        !files.every(
          (file) =>
            typeof file?.path === "string" && typeof file.name === "string",
        )
      ) {
        throw new Error(`${command} returned an invalid result.`);
      }
      return files;
    },
    examUploadClearFiles: () => invokeCommand("exam_upload_clear_files"),
    examOpenFile: (path) => invokeCommand("exam_open_file", { path }),
    examRetainFile: (id, path) =>
      booleanCommand("exam_retain_file", { id, path }),
    examSaveCopy: (id, path) => result("exam_save_copy", { id, path }),
  };
}
