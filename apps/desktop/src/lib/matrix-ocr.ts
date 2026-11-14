import { t } from "../i18n";
import { Channel, isTauri } from "@tauri-apps/api/core";
import type { MatrixVote } from "../generated/persistence";
import { invoke } from "../platform/native-error";

export const MATRIX_ROWS = 7;
export const MATRIX_COLUMNS = 10;
export const MATRIX_SIZE = MATRIX_ROWS * MATRIX_COLUMNS;
export type OcrProgress = { completed: number; total: number };

export function normalizeMatrixText(value: string): string {
  return value
    .toUpperCase()
    .replace(/[^A-Z]/g, "")
    .slice(0, MATRIX_SIZE);
}

/** Core reads a complete pasted Matrix into the editor's row order: letters
 * along rows, or titech-app's export (a JSON array listed down columns,
 * `fromTitechApp`). A bad paste is rejected instead of silently shifted. */
export async function parseMatrixText(
  text: string,
): Promise<{ cells: string[]; fromTitechApp: boolean }> {
  let pasted: { cells: string; order: "rows" | "columns" };
  try {
    pasted = await invoke("parse_school_matrix", { text });
  } catch {
    throw new Error(t("settings.credentialImport.pasteExactly70LettersIn"));
  }
  return {
    cells: [...pasted.cells],
    fromTitechApp: pasted.order === "columns",
  };
}

const MAX_IMAGE_BYTES = 20 * 1024 * 1024;

async function base64(blob: Blob): Promise<string> {
  let buffer: ArrayBuffer;
  try {
    buffer = await blob.arrayBuffer();
  } catch {
    // Whichever step reads the bytes first, an unreadable file gets the same
    // words, never the browser's own detail.
    throw new Error(t("matrixScan.cannotReadThisImageChoose"));
  }
  const bytes = new Uint8Array(buffer);
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
): Promise<MatrixVote[]> {
  if (!isTauri()) throw new Error(t("matrixScan.scanImageIsAvailableIn"));
  const image = await prepareMatrixImage(file);
  const onProgressChannel = new Channel<OcrProgress>();
  onProgressChannel.onmessage = (progress) =>
    onProgress?.(progress.completed, progress.total);
  return invoke<MatrixVote[]>("recognize_matrix_image", {
    image,
    onProgress: onProgressChannel,
  });
}
