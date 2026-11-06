import { t, type MessageKey } from "../../i18n";
import type { PrinterAccount } from "../../platform/desktop-bridge";

const ACCOUNT_TEXT: Record<
  Exclude<PrinterAccount["kind"], "ready">,
  MessageKey
> = {
  "not-connected": "tools.campusPrinter.saveTheTokyoTechAccount",
  "not-student-id": "tools.campusPrinter.accountIsNotAStudentId",
  "needs-attention": "tools.campusPrinter.accountNeedsAttention",
  unverified: "tools.campusPrinter.accountNotVerified",
};

/** Whose jobs the printer bills, or why there is no student ID. */
export const accountText = (account: PrinterAccount) =>
  account.kind === "ready"
    ? t("tools.campusPrinter.billedTo", { id: account.studentId })
    : t(ACCOUNT_TEXT[account.kind]);
