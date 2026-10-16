import { MantineProvider } from "@mantine/core";
import { useState } from "react";
import { appTheme, cssVariablesResolver } from "../../app/theme";
import { seedWorkspace, type Course, type Material } from "../../lib/model";
import type { LmsCourseRecord } from "../../platform/desktop-bridge";
import { Today } from "../../pages/today/TodayPage";
import { LmsWorkspace, type CourseEntry } from "./LmsWorkspace";
import { TodayWorkbench } from "./TodayWorkbench";

/** Development-only, isolated sample data: never imported into the workspace. */
export function LmsPreview() {
  const [data] = useState(() => {
    const workspace = seedWorkspace();
    workspace.deviceId = "lms-ux-preview";
    workspace.selectedTerm = "2026-3Q";
    const meta = {
      rev: 1,
      updatedAt: "2026-10-05T00:00:00Z",
      updatedBy: "preview",
      source: { provider: "lms" as const, externalId: "8432" },
    };
    const course: Course = {
      ...meta,
      id: "preview-automata",
      kind: "course",
      termId: "2026-3Q",
      code: "CSC.T251",
      title: "オートマトンと形式言語 / Automata and Formal Languages",
      instructor: "",
      credits: 2,
      creditSplit: { lecture: 2, exercise: 0, lab: 0 },
      instructionLanguage: "ja",
      color: "#476785",
      meetings: [
        {
          days: [1],
          periods: [3, 4],
          startTime: "10:45",
          endTime: "12:25",
          room: "Preview",
        },
      ],
    };
    workspace.courses = [course];
    workspace.memberships = [
      {
        id: "preview-member",
        kind: "membership",
        rev: 1,
        updatedAt: meta.updatedAt,
        updatedBy: "preview",
        termId: course.termId,
        courseId: course.id,
        status: "lms-member",
        evidence: meta.source,
      },
    ];
    workspace.materials = [
      ...Array.from({ length: 14 }, (_, i): Material => ({
        ...meta,
        id: `preview-file-${i + 1}`,
        kind: "material",
        courseId: course.id,
        sectionNumber: i + 1,
        sectionTitle: `第${i + 1}回`,
        session: i + 1,
        title: `講義資料 · ${i + 1}`,
        type: "link",
        module: "page",
        bodyHtml: `<h3>Preview · lecture ${i + 1}</h3><p>This sample verifies the reading area and tool panel. It is not a downloaded lecture file.</p><pre><code>Q = {q0, q1}\nΣ = {0, 1}\nδ : Q × Σ → Q</code></pre>`,
      })),
      ...["oop2026-schedule", "discord"].map((name, i): Material => ({
        ...meta,
        id: `preview-label-${i}`,
        kind: "material",
        courseId: course.id,
        sectionNumber: 0,
        sectionTitle: "一般",
        session: 0,
        title: name,
        type: "link",
        module: "label",
        bodyHtml: `<div class="no-overflow"><p><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/${223879 + i * 2}/mod_label/intro/${name}.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/${223879 + i * 2}/mod_label/intro/s_${name}.png" alt="${name}.png"></a></p></div>`,
      })),
      {
        ...meta,
        id: "preview-attachments",
        kind: "material",
        courseId: course.id,
        sectionNumber: 1,
        sectionTitle: "第1回",
        session: 1,
        title: "第1回基礎課題 · attachments",
        type: "link",
        module: "page",
        bodyHtml: `<div><ul><li><div><div class="fileuploadsubmission"><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/225786/mod_assign/introattachment/0/OOP-2026-1A-problem.zip?forcedownload=1">OOP-2026-1A-problem.zip</a></div><div class="fileuploadsubmissiontime">2026年 10月 5日 12:14</div></div></li><li><div><div class="fileuploadsubmission"><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/225787/mod_assign/introattachment/0/OOP-2026-1B-problem.zip?forcedownload=1">OOP-2026-1B-problem.zip</a></div><div class="fileuploadsubmissiontime">2026年 10月 5日 12:15</div></div></li></ul></div>`,
      },
      ...["slides-week1.pdf", "state-diagram.png", "exercise.pptx", "starter-code.zip", "recording.mp4"].map((name, i): Material => ({
        ...meta,
        id: `preview-resource-${i}`,
        kind: "material",
        courseId: course.id,
        sectionNumber: 1,
        sectionTitle: "第1回",
        session: 1,
        title: name.replace(/\.\w+$/, ""),
        type: "link",
        module: "resource",
        resourceUrl: `https://lms.s.isct.ac.jp/2025/pluginfile.php/0/mod_resource/content/1/${name}`,
      })),
      {
        ...meta,
        id: "preview-discussion",
        kind: "material",
        courseId: course.id,
        sectionNumber: 0,
        session: 0,
        sectionTitle: "一般",
        title: "Course announcements",
        type: "link",
        module: "forum",
        bodyHtml:
          '<article id="announcement-preview"><h5>Week 1 · Reading guide</h5><time>16 October</time><p>Review the lecture notes before the next class.</p><h3>State transitions</h3><p>A finite automaton moves between states as it reads each symbol. Start with the diagram, then trace one input at a time.</p><ul><li>Identify the initial and accepting states.</li><li>Follow the transition for each input symbol.</li><li>Compare the final state with the accepting set.</li></ul><p>This is isolated preview content.</p></article>',
      },
    ];
    workspace.assignments = [
      {
        ...meta,
        id: "preview-assignment",
        kind: "assignment",
        courseId: course.id,
        title: "Practice report · Preview",
        dueAt: "2026-10-08T08:00:00Z",
        details:
          "Sample requirement: explain a state transition diagram. The next action opens the official LMS interface; this preview does not submit anything.",
        submission: "open",
      },
    ];
    return workspace;
  });
  // Opens on the sample course, the surface this preview exists for.
  const [entry, setEntry] = useState<CourseEntry | null>({ courseId: "preview-automata", request: 1 });
  const [filtered, setFiltered] = useState(false);
  const course = data.courses[0];
  const summaries = [
    "句構造文法",
    "正規文法",
    "有限オートマトン",
    "正規文法と有限オートマトン",
    "ε遷移を含むオートマトン",
    "決定性有限オートマトンの最小化",
    "正規言語の反復補題",
    "正規表現とオートマトン",
    "復習",
    "文脈自由文法",
    "チョムスキーの標準形",
    "文脈自由言語の反復補題",
    "NPDA / DPDA",
    "文法とプッシュダウンオートマトン",
  ];
  const record: LmsCourseRecord = {
    id: "preview-lms",
    lmsId: 8432,
    shortname: "Preview[2026]",
    title: { ja: course.title },
    quarters: ["3Q"],
    url: "https://lms.s.isct.ac.jp/2025/course/view.php?id=8432",
    sections: [
      {
        id: 0,
        number: 0,
        title: "一般",
        visible: true,
        bodyHtml:
          "<p>Preview data · this view does not modify your courses or contact LMS.</p>",
      },
      ...summaries.map((title, i) => ({
        id: i + 1,
        number: i + 1,
        title: `第${i + 1}回`,
        visible: true,
        bodyHtml:
          i === 12
            ? "<p>授業計画：非決定性プッシュダウンオートマトン(NPDA)，決定性プッシュダウンオートマトン(DPDA)<br>課題：状態遷移図，受理する言語，時点表示<br>Course schedule: nondeterministic pushdown automaton (NPDA), deterministic pushdown automaton (DPDA)<br>Objectives: state-transition diagram, languages accepted, instantaneous description</p>"
            : `<p>授業計画：${title}<br>課題：授業内容を確認する<br>Course schedule: ${i + 1}. Preview lesson<br>Objectives: review the lecture content</p>`,
      })),
    ],
  };
  return (
    <MantineProvider
      theme={appTheme}
      cssVariablesResolver={cssVariablesResolver}
    >
      <div className="lmsPreviewShell">
        <div className="lmsPreviewNotice">
          <span>UX preview · isolated sample data</span>
          <label>
            <input
              type="checkbox"
              checked={filtered}
              onChange={(e) => setFiltered(e.target.checked)}
            />{" "}
            Bilingual filter
          </label>
        </div>
        <div className="lmsPreviewMain">
          <TodayWorkbench
            entry={entry}
            renderWorkspace={(entry, active) => (
              <LmsWorkspace
                course={course}
                data={{
                  ...data,
                  settings: { ...data.settings, lmsBilingualFilter: filtered },
                }}
                record={record}
                entry={entry}
                active={active}
                navigationProvided
                schoolWebAvailable={false}
                close={() => setEntry(null)}
              />
            )}
          >
            {(compact) => (
              <Today
                compact={compact}
                selection={entry ?? undefined}
                data={data}
                term="2026-3Q"
                open={(courseId, _tab, context) =>
                  setEntry((previous) =>
                    previous?.courseId === courseId && !previous.assignmentId
                      ? null
                      : { courseId, request: Date.now(), ...context },
                  )
                }
                openTask={(assignmentId) =>
                  setEntry((previous) =>
                    previous?.assignmentId === assignmentId
                      ? null
                      : {
                          courseId: course.id,
                          assignmentId,
                          request: Date.now(),
                        },
                  )
                }
                openEvent={() => {}}
                openPlan={() => {}}
                hideAssignment={() => {}}
                openSchedule={() => {}}
              />
            )}
          </TodayWorkbench>
        </div>
      </div>
    </MantineProvider>
  );
}
