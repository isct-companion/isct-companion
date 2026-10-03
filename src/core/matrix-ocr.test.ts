// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  bitmap: { width: 3000, height: 2000, close: vi.fn() },
  drawImage: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  ...mocks,
  Channel: class {
    onmessage = (_value: number) => {};
  },
}));
import {
  recognizeMatrixImage,
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixFile,
} from "./matrix-ocr";
beforeEach(() => {
  vi.clearAllMocks();
  mocks.isTauri.mockReturnValue(true);
  vi.stubGlobal(
    "createImageBitmap",
    vi.fn(async () => mocks.bitmap),
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    fillStyle: "",
    fillRect: vi.fn(),
    drawImage: mocks.drawImage,
  } as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockImplementation(
    function () {
      return "data:image/png;base64," + btoa(`${this.width}x${this.height}`);
    },
  );
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("sends only the selected image to native OCR and forwards progress", async () => {
  const results = [{ text: "O", confidence: 0.89 }];
  mocks.invoke.mockImplementation(async (name, payload) => {
    expect(name).toBe("recognize_matrix_image");
    expect(atob(payload.data)).toBe("2200x1467");
    expect(atob(payload.preview)).toBe("600x400");
    payload.onProgress.onmessage(35);
    return results;
  });
  const progress = vi.fn();
  expect(
    await recognizeMatrixImage(new File(["image-data"], "card.png"), progress),
  ).toEqual(results);
  expect(progress).toHaveBeenCalledWith(35, 70);
  expect(mocks.invoke).toHaveBeenCalledTimes(1);
  expect(mocks.bitmap.close).toHaveBeenCalledTimes(1);
  expect(mocks.drawImage).toHaveBeenNthCalledWith(
    1,
    mocks.bitmap,
    0,
    0,
    2200,
    1467,
  );
  expect(mocks.drawImage).toHaveBeenNthCalledWith(
    2,
    expect.any(HTMLCanvasElement),
    0,
    0,
    600,
    400,
  );
});
it("surfaces native errors without discarding their message", async () => {
  mocks.invoke.mockRejectedValue("Could not locate all 7 × 10 cells.");
  await expect(
    recognizeMatrixImage(new File(["data"], "card.png")),
  ).rejects.toThrow("Could not locate");
});
it("rejects web previews and invalid file sizes before IPC", async () => {
  mocks.isTauri.mockReturnValue(false);
  await expect(
    recognizeMatrixImage(new File(["data"], "card.png")),
  ).rejects.toThrow("desktop app");
  mocks.isTauri.mockReturnValue(true);
  await expect(recognizeMatrixImage(new File([], "empty.png"))).rejects.toThrow(
    "20 MB",
  );
  await expect(
    recognizeMatrixImage({ size: 21 * 1024 * 1024 } as File),
  ).rejects.toThrow("20 MB");
  expect(mocks.invoke).not.toHaveBeenCalled();
});
it("keeps review bands and manual matrix normalization", () => {
  expect(matrixConfidence("A", 0.95)).toBe("high");
  expect(matrixConfidence("O", 0.89)).toBe("medium");
  expect(matrixConfidence("I", 0.74)).toBe("low");
  expect(matrixConfidence("", 0.99)).toBe("low");
  expect(matrixConfidence("A", NaN)).toBe("low");
  expect(normalizeMatrixText("a, b 0 1 8")).toBe("AB");
});

it("releases the image bitmap when canvas preparation fails", async () => {
  vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);
  await expect(
    recognizeMatrixImage(new File(["data"], "card.png")),
  ).rejects.toThrow("Cannot prepare");
  expect(mocks.bitmap.close).toHaveBeenCalledOnce();
  expect(mocks.invoke).not.toHaveBeenCalled();
});

// Same card as the backend export test: cell i (row by row) holds the i-th letter.
const rowMajor = Array.from({ length: 70 }, (_, i) =>
  String.fromCharCode(65 + (i % 26)),
);
const matrixFile = Array.from(
  { length: 70 },
  (_, k) => rowMajor[(k % 7) * 10 + Math.floor(k / 7)],
);
it("reads Matrix files column by column into row-by-row cells", () => {
  expect(matrixFile.slice(0, 8)).toEqual([
    "A",
    "K",
    "U",
    "E",
    "O",
    "Y",
    "I",
    "B",
  ]);
  expect(parseMatrixFile(JSON.stringify(matrixFile, null, 2))).toEqual(
    rowMajor,
  );
  expect(
    parseMatrixFile(
      JSON.stringify(matrixFile.map((cell) => cell.toLowerCase())),
    ),
  ).toEqual(rowMajor);
});
it.each([
  ["invalid JSON", "[A, B]"],
  ["an object", JSON.stringify({ matrix: matrixFile })],
  ["69 cells", JSON.stringify(matrixFile.slice(1))],
  ["a digit", JSON.stringify(["1", ...matrixFile.slice(1)])],
  ["two letters in a cell", JSON.stringify(["AB", ...matrixFile.slice(1)])],
])("rejects a Matrix file with %s", (_, text) => {
  expect(() => parseMatrixFile(text)).toThrow("JSON array of 70 letters");
});
