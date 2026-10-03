import { Channel, invoke, isTauri } from "@tauri-apps/api/core";

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

export function normalizeMatrixText(value: string): string {
  return value
    .toUpperCase()
    .replace(/[^A-Z]/g, "")
    .slice(0, MATRIX_SIZE);
}

/** Matrix files list the card column by column (A1…A7, B1…B7, …) as a JSON
 * array of 70 letters; cells here run row by row. Mirrors `matrix_file_json`
 * in backend/src/school_auth.rs. */
export function parseMatrixFile(text: string): string[] {
  let cells: unknown;
  try {
    cells = JSON.parse(text);
  } catch {
    cells = undefined;
  }
  if (
    !Array.isArray(cells) ||
    cells.length !== MATRIX_SIZE ||
    !cells.every((cell) => typeof cell === "string" && /^[A-Z]$/i.test(cell))
  )
    throw new Error(
      "A Matrix file is a JSON array of 70 letters, column by column (A1…A7, B1…B7, …).",
    );
  return Array.from({ length: MATRIX_SIZE }, (_, index) => {
    const row = Math.floor(index / MATRIX_COLUMNS);
    const column = index % MATRIX_COLUMNS;
    return (cells[column * MATRIX_ROWS + row] as string).toUpperCase();
  });
}

/** Keep the established WebView decoding/resampling; native code consumes these
 * exact PNG pixels without resizing again. Inference and filters remain Rust. */
export async function prepareMatrixCanvases(file: File) {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, 2200 / Math.max(bitmap.width, bitmap.height));
  const source = document.createElement("canvas");
  source.width = Math.max(1, Math.round(bitmap.width * scale));
  source.height = Math.max(1, Math.round(bitmap.height * scale));
  try {
    const context = source.getContext("2d");
    if (!context) throw new Error("Cannot prepare the selected image.");
    context.fillStyle = "white";
    context.fillRect(0, 0, source.width, source.height);
    context.drawImage(bitmap, 0, 0, source.width, source.height);
  } finally {
    bitmap.close();
  }
  const smallScale = Math.min(1, 600 / Math.max(source.width, source.height));
  const preview = document.createElement("canvas");
  preview.width = Math.max(1, Math.round(source.width * smallScale));
  preview.height = Math.max(1, Math.round(source.height * smallScale));
  const context = preview.getContext("2d");
  if (!context) throw new Error("Cannot prepare the matrix preview.");
  context.drawImage(source, 0, 0, preview.width, preview.height);
  return {
    data: source.toDataURL("image/png").split(",")[1],
    preview: preview.toDataURL("image/png").split(",")[1],
  };
}

export async function recognizeMatrixImage(
  file: File,
  onProgress?: (completed: number, total: number) => void,
): Promise<Recognition[]> {
  if (!isTauri())
    throw new Error("Scan image is available in the desktop app.");
  if (!file.size || file.size > 20 * 1024 * 1024)
    throw new Error("Choose a valid image smaller than 20 MB.");
  const images = await prepareMatrixCanvases(file);
  const onProgressChannel = new Channel<number>();
  onProgressChannel.onmessage = (completed) =>
    onProgress?.(completed, MATRIX_SIZE);
  try {
    return await invoke<Recognition[]>("recognize_matrix_image", {
      ...images,
      onProgress: onProgressChannel,
    });
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}
