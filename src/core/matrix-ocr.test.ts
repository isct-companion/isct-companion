import { expect, it } from "vitest";
import {
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
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
