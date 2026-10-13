// @vitest-environment jsdom
import { expect, it } from "vitest";
import { assignmentAttachments, assignmentBrief } from "./assignment-workspace";

const html = `<p>Implement the classes.</p><ul><li>Use Java 21</li><li>Write tests</li></ul>
<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip">problem</a>
<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip">again</a>
<a href="https://example.com/pluginfile.php/evil.zip">elsewhere</a>`;

it("collects each LMS attachment once, named by its file", () => {
  expect(assignmentAttachments(html)).toEqual([
    { url: "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip", name: "OOP-2026-1B-problem.zip" },
  ]);
});

it("writes a brief with the facts, the description and the attachments", () => {
  const brief = assignmentBrief({ title: "1B", course: "OOP", due: "2026/10/20 23:59 JST", html, attachments: ["OOP-2026-1B-problem.zip"] });
  expect(brief).toContain("# 1B\n\n- Course: OOP\n- Due: 2026/10/20 23:59 JST\n");
  expect(brief).toContain("Implement the classes.\n- Use Java 21\n- Write tests");
  expect(brief).toContain("## Attachments\n\n- OOP-2026-1B-problem.zip\n");
});
