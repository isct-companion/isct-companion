// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { imageOnly, safeContent } from "./content";
const enrich = new Function(
  `return ${readFileSync(`${process.cwd()}/backend/src/lms_course_content.js`, "utf8")}`,
)() as (
  page: Document,
  state: { section: unknown[]; cm: any[] },
  url: string,
  bounded: (body: string) => string,
) => void;
it("extracts image-only labels across different theme wrappers, preserving the full image target", () => {
  for (const wrapper of [
    "activity-altcontent",
    "contentwithoutlink",
    "custom-theme",
  ]) {
    const page = new DOMParser().parseFromString(
      `<div id="section-0"><h3><a href="/2025/course/section.php?id=9">General</a></h3><button>Collapse</button><p>Teacher summary</p><div id="module-10"><input><label>Choose activity</label><div class="${wrapper}"><a href="/2025/pluginfile.php/1/mod_label/intro/full.png"><img src="/2025/pluginfile.php/1/mod_label/intro/s_full.png" alt="schedule.png"></a></div></div></div>`,
      "text/html",
    );
    const state = {
      section: [{ id: "9", number: 0 }],
      cm: [{ id: "10", module: "label", sectionid: "9" }],
    };
    enrich(
      page,
      state,
      "https://lms.s.isct.ac.jp/2025/course/view.php?id=42",
      (body) => body,
    );
    const body = (state.cm[0] as any).bodyHtml;
    expect(body).toContain("/intro/full.png");
    expect(body).toContain("/intro/s_full.png");
    expect(body).not.toContain("Choose activity");
    const inert = new DOMParser().parseFromString(
      safeContent(body),
      "text/html",
    );
    expect(inert.querySelectorAll("a")).toHaveLength(1);
    expect(inert.querySelector("a")?.href).toContain("/intro/full.png");
    expect(inert.body.textContent).toContain("schedule.png");
    expect((state.section[0] as any).bodyHtml).toContain("Teacher summary");
    expect((state.section[0] as any).bodyHtml).not.toContain("module-10");
  }
});
it("preserves arbitrary summary tables and activity descriptions but removes Moodle navigation and icons", () => {
  const page = new DOMParser().parseFromString(
    '<section id="section-2"><h3><a href="/2025/course/section.php?id=21">Lesson</a></h3><table><tr><td>Office hours</td></tr></table><article id="module-30"><img src="/2025/theme/image.php/remui/core/icon"><a href="/2025/mod/resource/view.php?id=30">File title</a><aside><p>Teacher description</p><a href="/2025/pluginfile.php/3/a.pdf">Attachment</a></aside></article></section>',
    "text/html",
  );
  const state = {
    section: [{ id: "21", number: 2 }],
    cm: [
      {
        id: "30",
        url: "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=30",
      },
    ],
  };
  enrich(
    page,
    state,
    "https://lms.s.isct.ac.jp/2025/course/view.php?id=42",
    (body) => body,
  );
  expect((state.section[0] as any).bodyHtml).toContain("Office hours");
  expect((state.section[0] as any).bodyHtml).not.toContain(
    "Teacher description",
  );
  expect((state.cm[0] as any).bodyHtml).toContain("Teacher description");
  expect((state.cm[0] as any).bodyHtml).not.toContain("File title");
  expect((state.cm[0] as any).bodyHtml).not.toContain("theme/image.php");
});
it("extracts announcement posts and attachments without importing the discussion table or reply controls", () => {
  const forum = new Function(`return ${readFileSync(`${process.cwd()}/backend/src/lms_forum_content.js`, "utf8")}`)();
  const page = new DOMParser().parseFromString('<main><table><tr><td>Listing</td></tr></table><article id="p51978"><header><h3>Classroom change</h3><time>30 September</time></header><div id="post-content-51978"><p>Room W3-707</p><img src="/2025/pluginfile.php/1/map.png" alt="Map"></div><a href="/2025/pluginfile.php/2/notice.pdf">Notice PDF</a><button>Reply</button></article></main>', 'text/html');
  const html = safeContent(forum(page, 'https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=33638'));
  expect(html).toContain('Classroom change');
  expect(html).toContain('Room W3-707');
  expect(html).toContain('lmsAnnouncement');
  expect(html).toContain('https://lms.s.isct.ac.jp/2025/pluginfile.php/2/notice.pdf');
  expect(html).not.toContain('<table');
  expect(html).not.toContain('Reply');
});
it("filters LMS expand/collapse chrome with an empty fragment", () => {
  const page = new DOMParser().parseFromString('<div id="section-0"><a href="/2025/course/view.php?id=42#">Collapse all Expand all</a><p>Teacher note</p></div>', 'text/html');
  const state = {section:[{id:9,number:0}],cm:[]};
  enrich(page,state,'https://lms.s.isct.ac.jp/2025/course/view.php?id=42',(s)=>s);
  expect((state.section[0] as any).bodyHtml).not.toContain('Collapse all');
  expect((state.section[0] as any).bodyHtml).toContain('Teacher note');
});
it("also filters Moodle collapse controls from old cached HTML at rendering", () => {
  expect(safeContent('<p>Teacher text</p><a href="https://lms.s.isct.ac.jp/2025/course/view.php?id=11677#">すべてを折りたたむ すべてを展開する</a>')).not.toContain('すべてを折りたたむ');
});

it("creates thumbnail targets for plain image links as well as embedded images", () => {
  for (const extension of ["png", "jpg", "webp", "gif", "svg", "avif"]) {
    const url = `https://lms.s.isct.ac.jp/2025/pluginfile.php/223879/mod_label/intro/schedule.${extension}?forcedownload=1`;
    const parsed = new DOMParser().parseFromString(safeContent(`<a href="${url}">schedule.${extension}</a>`), "text/html");
    expect(parsed.querySelector(".lmsImageCard [data-image-src]")?.getAttribute("data-image-src")).toBe(url);
    expect(parsed.querySelector("a")?.textContent).toBe(`schedule.${extension}`);
  }
});

it("turns Moodle attachment lists into a card grid with type and upload time", () => {
  const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/225786/mod_assign/introattachment/0/OOP-2026-1A-problem.zip?forcedownload=1";
  const html = `<div><ul><li><div><div class="fileuploadsubmission"><img class="icon" alt="zip" src="https://lms.s.isct.ac.jp/2025/theme/image.php/remui/core/1/f/archive"> <a href="${url}">OOP-2026-1A-problem.zip</a></div><div class="fileuploadsubmissiontime">2026年 10月 5日 12:14</div></div></li></ul></div>`;
  const parsed = new DOMParser().parseFromString(safeContent(html), "text/html");
  expect(parsed.querySelector("ul, li")).toBeNull();
  const card = parsed.querySelector(".lmsFileList > a.lmsAttachmentCard")!;
  expect(card.getAttribute("data-family")).toBe("archive");
  expect(card.querySelector(".lmsAttachmentName")?.textContent).toBe("OOP-2026-1A-problem.zip");
  expect(card.querySelector("small")?.textContent).toBe("ZIP · 2026年 10月 5日 12:14");
});

it("keeps ordinary lists that mix text and links as lists", () => {
  const parsed = new DOMParser().parseFromString(safeContent('<ul><li>Read chapter 2</li><li><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf">Slides</a></li></ul>'), "text/html");
  expect(parsed.querySelectorAll("li")).toHaveLength(2);
});

it("keeps the thumbnail of an image wrapped in its full-size link", () => {
  const full = "https://lms.s.isct.ac.jp/2025/pluginfile.php/223879/mod_label/intro/oop2026-schedule.png";
  const small = "https://lms.s.isct.ac.jp/2025/pluginfile.php/223879/mod_label/intro/s_oop2026-schedule.png";
  const parsed = new DOMParser().parseFromString(safeContent(`<a href="${full}"><img src="${small}" alt="schedule"></a>`), "text/html");
  expect(parsed.querySelector("a.lmsImageCard [data-image-src]")?.getAttribute("data-image-src")).toBe(small);
  expect(parsed.querySelector(".lmsAttachmentName")).toBeNull();
});

it("recognises image-only labels but not labels with text", () => {
  const image = '<a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/s_a.png" alt="A"></a>';
  expect(imageOnly(`<div><p>${image}</p></div>`)).toBe(true);
  expect(imageOnly(`<p>Week 1 schedule</p>${image}`)).toBe(false);
  expect(imageOnly("<p>No images</p>")).toBe(false);
});
