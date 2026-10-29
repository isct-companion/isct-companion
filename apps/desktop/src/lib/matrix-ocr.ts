import { t } from "../i18n";
import { Channel, isTauri } from "@tauri-apps/api/core";
import { invoke } from "../platform/native-error";

export const MATRIX_ROWS = 7;
export const MATRIX_COLUMNS = 10;
export const MATRIX_SIZE = MATRIX_ROWS * MATRIX_COLUMNS;
export type MatrixConfidence = "high" | "medium" | "low";
export function matrixConfidence(
  text: string,
  confidence: number,
): MatrixConfidence {
  if (!text || !Number.isFinite(confidence) || confidence < 0.75) return "low";
  return confidence >= 0.9 ? "high" : "medium";
}
export type Recognition = { text: string; confidence: number };
export type OcrProgress = { completed: number; total: number };

export function normalizeMatrixText(value: string): string {
  return value
    .toUpperCase()
    .replace(/[^A-Z]/g, "")
    .slice(0, MATRIX_SIZE);
}

/** Reads a complete Matrix in the editor's visual row order (A1…J1, A2…).
 * Spaces, tabs and line breaks are formatting only; everything else must be a
 * letter so a bad paste is rejected instead of silently shifted or truncated. */
export function parseMatrixText(text: string): string[] {
  const compact = text.toUpperCase().replace(/\s/g, "");
  if (!/^[A-Z]{70}$/.test(compact))
    throw new Error(t("settings.credentialImport.pasteExactly70LettersIn"));
  return [...compact];
}

const MAX_IMAGE_BYTES = 20 * 1024 * 1024;

async function base64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const chunks: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    chunks.push(
      String.fromCharCode(...bytes.subarray(offset, offset + 0x8000)),
    );
  }
  return btoa(chunks.join(""));
}

/** HEIC conversion is a shell codec capability. Decode, orientation, alpha
 * compositing and both OCR working sizes belong to the Rust library. */
export async function prepareMatrixImage(file: File): Promise<string> {
  if (!file.size || file.size > MAX_IMAGE_BYTES)
    throw new Error(t("matrixScan.chooseAValidImageSmaller"));
  const { isHeic, heicTo } = await import("heic-to/csp");
  let input: Blob = file;
  let heic = false;
  try {
    heic = await isHeic(file);
  } catch {
    throw new Error(t("matrixScan.cannotReadThisImageChoose"));
  }
  if (heic) {
    try {
      input = await heicTo({ blob: file, type: "image/jpeg", quality: 0.95 });
    } catch {
      throw new Error(t("matrixScan.cannotDecodeThisHEICImage"));
    }
    if (!input.size || input.size > MAX_IMAGE_BYTES)
      throw new Error(t("matrixScan.decodedHEICImageExceeds20"));
  }
  return base64(input);
}

export async function recognizeMatrixImage(
  file: File,
  onProgress?: (completed: number, total: number) => void,
): Promise<Recognition[]> {
  if (!isTauri()) throw new Error(t("matrixScan.scanImageIsAvailableIn"));
  const image = await prepareMatrixImage(file);
  const onProgressChannel = new Channel<OcrProgress>();
  onProgressChannel.onmessage = (progress) =>
    onProgress?.(progress.completed, progress.total);
  try {
    return await invoke<Recognition[]>("recognize_matrix_image", {
      image,
      onProgress: onProgressChannel,
    });
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}
