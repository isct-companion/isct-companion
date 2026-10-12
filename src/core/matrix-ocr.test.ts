import { afterEach, expect, it, vi } from "vitest";
import {
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  prepareMatrixCanvases,
} from "./matrix-ocr";

it("keeps review bands and manual matrix normalization", () => {
  expect(matrixConfidence("A", 0.95)).toBe("high");
  expect(matrixConfidence("O", 0.89)).toBe("medium");
  expect(matrixConfidence("I", 0.74)).toBe("low");
  expect(matrixConfidence("", 0.99)).toBe("low");
  expect(matrixConfidence("A", NaN)).toBe("low");
  expect(normalizeMatrixText("a, b 0 1 8")).toBe("AB");
});

// Same card as the backend clipboard test: cell i (row by row) holds the i-th letter.
const rowMajor = Array.from({ length: 70 }, (_, i) =>
  String.fromCharCode(65 + (i % 26)),
);
it("reads whitespace-formatted Matrix text in visual row order", () => {
  const rows = Array.from({ length: 7 }, (_, row) =>
    rowMajor.slice(row * 10, row * 10 + 10).join(" "),
  ).join("\n");
  expect(parseMatrixText(rows)).toEqual(rowMajor);
  expect(parseMatrixText(rowMajor.join("").toLowerCase())).toEqual(rowMajor);
});
it.each([
  ["69 letters", rowMajor.slice(1).join("")],
  ["71 letters", `${rowMajor.join("")}A`],
  ["punctuation", `${rowMajor.slice(0, 69).join("")}-`],
])("rejects Matrix text with %s", (_, text) => {
  expect(() => parseMatrixText(text)).toThrow("exactly 70 letters");
});

const heic = vi.hoisted(() => ({ isHeic: vi.fn(), heicTo: vi.fn() }));
vi.mock("heic-to/csp", () => heic);
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  heic.isHeic.mockReset();
  heic.heicTo.mockReset();
});
function imagePreparation() {
  const bitmap = { width: 4000, height: 3000, close: vi.fn() };
  const drawImage = vi.fn();
  const context = { fillRect: vi.fn(), drawImage, fillStyle: "" };
  vi.stubGlobal("document", {
    createElement: () => ({
      width: 0,
      height: 0,
      getContext: () => context,
      toDataURL: () => "data:image/png;base64,prepared",
    }),
  });
  return { bitmap, drawImage };
}
it("preserves the built-in decoder for supported images", async () => {
  const { bitmap, drawImage } = imagePreparation();
  vi.stubGlobal("createImageBitmap", vi.fn().mockResolvedValue(bitmap));
  await expect(
    prepareMatrixCanvases(new File(["png"], "photo.png")),
  ).resolves.toEqual({ data: "prepared", preview: "prepared" });
  expect(drawImage.mock.calls[0].slice(1)).toEqual([0, 0, 2200, 1650]);
  expect(bitmap.close).toHaveBeenCalledOnce();
  expect(heic.isHeic).not.toHaveBeenCalled();
});
it("decodes HEIC by contents when the WebView rejects it", async () => {
  const { bitmap } = imagePreparation();
  vi.stubGlobal(
    "createImageBitmap",
    vi.fn().mockRejectedValue(new Error("os error")),
  );
  heic.isHeic.mockResolvedValue(true);
  heic.heicTo.mockResolvedValue(bitmap);
  const file = new File(["heic"], "photo", { type: "" });
  await prepareMatrixCanvases(file);
  expect(heic.heicTo).toHaveBeenCalledWith({ blob: file, type: "bitmap" });
  expect(bitmap.close).toHaveBeenCalledOnce();
});
it("reports unreadable files without leaking a codec OS error", async () => {
  imagePreparation();
  vi.stubGlobal(
    "createImageBitmap",
    vi.fn().mockRejectedValue(new Error("os error")),
  );
  heic.isHeic.mockResolvedValue(false);
  await expect(
    prepareMatrixCanvases(new File(["bad"], "photo.png")),
  ).rejects.toThrow("Cannot read this image");
  heic.isHeic.mockResolvedValue(true);
  heic.heicTo.mockRejectedValue(new Error("decoder failed"));
  await expect(
    prepareMatrixCanvases(new File(["bad"], "photo.heic")),
  ).rejects.toThrow("Cannot decode this HEIC image");
});
