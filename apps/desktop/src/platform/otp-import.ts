import type { OtpImportAccount } from "../generated/persistence";
import { t } from "../i18n";
import { NativeError, invoke } from "./native-error";

export type { OtpImportAccount };
export function importOtp(payload: string): Promise<OtpImportAccount[]> {
  return invoke("import_school_otp", { payload });
}
export function otpImportError(error: unknown): string {
  return error instanceof NativeError
    ? error.message
    : t("error.otp-import-invalid");
}
export async function readOtpImage(file: File): Promise<string> {
  if (file.size > 10 * 1024 * 1024)
    throw new Error(t("otpImport.chooseAQRCodeImage"));
  const { BrowserQRCodeReader } = await import("@zxing/browser");
  const url = URL.createObjectURL(file);
  try {
    return (await new BrowserQRCodeReader().decodeFromImageUrl(url)).getText();
  } catch {
    throw new Error(t("settings.otp.noQRCodeFoundChoose"));
  } finally {
    URL.revokeObjectURL(url);
  }
}
