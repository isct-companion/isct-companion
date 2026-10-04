import { expect, it } from "vitest";
import {
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixFile,
} from "./matrix-ocr";

it("keeps review bands and manual matrix normalization", () => {
  expect(matrixConfidence("A", 0.95)).toBe("high");
  expect(matrixConfidence("O", 0.89)).toBe("medium");
  expect(matrixConfidence("I", 0.74)).toBe("low");
  expect(matrixConfidence("", 0.99)).toBe("low");
  expect(matrixConfidence("A", NaN)).toBe("low");
  expect(normalizeMatrixText("a, b 0 1 8")).toBe("AB");
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
  ["69 cells", JSON.stringify(matrixFile.slice(1))],
])("rejects a Matrix file with %s", (_, text) => {
  expect(() => parseMatrixFile(text)).toThrow("JSON array of 70 letters");
});
