import { afterEach, expect, it, vi } from "vitest";
const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class<T> {
    onmessage: (message: T) => void = () => undefined;
  },
  invoke: native.invoke,
  isTauri: native.isTauri,
}));
import {
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  prepareMatrixImage,
  recognizeMatrixImage,
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
  native.invoke.mockReset();
  native.isTauri.mockReset().mockReturnValue(true);
  heic.isHeic.mockReset();
  heic.heicTo.mockReset();
});
it("passes supported image bytes through without canvas preparation", async () => {
  heic.isHeic.mockResolvedValue(false);
  await expect(
    prepareMatrixImage(new File(["png"], "photo.png")),
  ).resolves.toBe("cG5n");
  expect(heic.heicTo).not.toHaveBeenCalled();
});
it("converts HEIC by contents to shell-provided JPEG bytes", async () => {
  heic.isHeic.mockResolvedValue(true);
  heic.heicTo.mockResolvedValue(new Blob(["jpeg"], { type: "image/jpeg" }));
  const file = new File(["heic"], "photo", { type: "" });
  await expect(prepareMatrixImage(file)).resolves.toBe("anBlZw==");
  expect(heic.heicTo).toHaveBeenCalledWith({
    blob: file,
    type: "image/jpeg",
    quality: 0.95,
  });
});
it("reports HEIC conversion failures and empty input without codec details", async () => {
  await expect(prepareMatrixImage(new File([], "empty.png"))).rejects.toThrow(
    "smaller than 20 MB",
  );
  heic.isHeic.mockResolvedValue(true);
  heic.heicTo.mockRejectedValue(new Error("decoder failed"));
  await expect(
    prepareMatrixImage(new File(["bad"], "photo.heic")),
  ).rejects.toThrow("Cannot decode this HEIC image");
});
it("reports HEIC inspection failures without codec details", async () => {
  heic.isHeic.mockRejectedValue(new Error("parser detail"));
  await expect(
    prepareMatrixImage(new File(["bad"], "photo.bin")),
  ).rejects.toThrow(
    "Cannot read this image. Choose a valid PNG, JPEG, WebP or HEIC photo.",
  );
});

it("sends one raw image and forwards typed native progress", async () => {
  heic.isHeic.mockResolvedValue(false);
  native.invoke.mockImplementation(async (_command, args) => {
    args.onProgress.onmessage({ completed: 12, total: 70 });
    return [];
  });
  const progress = vi.fn();
  await recognizeMatrixImage(new File(["raw"], "photo.jpg"), progress);
  expect(native.invoke).toHaveBeenCalledWith("recognize_matrix_image", {
    image: "cmF3",
    onProgress: expect.anything(),
  });
  expect(progress).toHaveBeenCalledWith(12, 70);
});
