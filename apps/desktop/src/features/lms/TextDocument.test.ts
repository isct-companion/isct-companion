// @vitest-environment jsdom
import { expect, it } from "vitest";
import { textLanguage } from "./TextDocument";
import { previewKind } from "./files";
it("recognizes school code attachments even with download queries and uppercase R", () => {
  for (const [extension, name] of [["c","C / C++"],["cpp","C / C++"],["java","Java"],["py","Python"],["R","R"],["txt","Plain text"]]) {
    const url = `https://lms.s.isct.ac.jp/2025/pluginfile.php/1/lecture.${extension}?forcedownload=1`;
    expect(previewKind(url)).toBe("text");
    expect(textLanguage(url).name).toBe(name);
  }
  expect(previewKind("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/notes.md")).toBe("markdown");
});
