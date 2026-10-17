import { invoke } from "@tauri-apps/api/core";
import type { OtpImportAccount } from "../generated/persistence";
import { t } from "../i18n";

export type { OtpImportAccount };
export function importOtp(payload: string): Promise<OtpImportAccount[]> {
  return invoke("import_school_otp", { payload });
}
export function otpImportError(error: unknown): string {
  const kind =
    error && typeof error === "object" && "kind" in error
      ? error.kind
      : undefined;
  if (kind === "multiplePages")
    return t("Export only your school account, then scan its QR code again.");
  if (kind === "unsupported")
    return t("This account uses an unsupported verification code format.");
  return t("This is not a supported authenticator QR code.");
}
export async function readOtpImage(file: File): Promise<string> {
  if (file.size > 10 * 1024 * 1024)
    throw new Error(t("Choose a QR code image smaller than 10 MB."));
  const { BrowserQRCodeReader } = await import("@zxing/browser");
  const url = URL.createObjectURL(file);
  try {
    return (await new BrowserQRCodeReader().decodeFromImageUrl(url)).getText();
  } catch {
    throw new Error(
      t("No QR code found. Choose a clear image containing the whole QR code."),
    );
  } finally {
    URL.revokeObjectURL(url);
  }
}
