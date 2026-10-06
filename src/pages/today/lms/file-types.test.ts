import { expect, it } from "vitest";
import { extension, fileFamily, previewKind } from "./file-types";

const lms = (name: string) => `https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_resource/content/1/${name}?forcedownload=1`;

it("reads the extension from encoded LMS file names", () => {
  expect(extension(lms("%E8%AC%9B%E7%BE%A9.PDF"))).toBe("pdf");
  expect(extension(lms("README"))).toBe("");
});

it("previews images, media and text in app, and downloads office files", () => {
  expect(previewKind(lms("photo.HEIC"))).toBe("image");
  expect(previewKind(lms("lecture.mp4"))).toBe("video");
  expect(previewKind(lms("talk.m4a"))).toBe("audio");
  expect(previewKind(lms("main.cpp"))).toBe("text");
  expect(previewKind(lms("slides.pptx"))).toBeUndefined();
  expect(previewKind(lms("notebook.ipynb"))).toBeUndefined();
});

it("gives every row a family, falling back to the Moodle module", () => {
  expect(fileFamily(lms("week1.pptx"))).toBe("slides");
  expect(fileFamily(lms("scores.xlsx"))).toBe("sheet");
  expect(fileFamily(lms("code.zip"))).toBe("archive");
  expect(fileFamily("", "folder")).toBe("folder");
  expect(fileFamily("", "url")).toBe("link");
  expect(fileFamily("", "resource")).toBe("file");
});
