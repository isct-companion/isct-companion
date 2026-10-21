import { submissionLabel } from "../../lib/academic";
import { ExamCards } from "../../components/exams/ExamCards";
import { Button, ScrollArea, Tabs } from "@mantine/core";
import { useState } from "react";
import { openExternalUrl } from "../../components/ExternalLink";
import { SyncRing, progressLabel } from "../../components/PullRing";
import type { PageProgress } from "../../platform/sync";
import { dayNames } from "../../components/timetable/timetable";
import {
  type ClassGroupChoice,
  type Course,
  type Workspace,
} from "../../lib/model";
import { tokyoDate } from "../../lib/civil-date";
import { notify } from "../../app/notify";
import { workspaceActions } from "../../store/workspace";
import { ClassGroupSwitch } from "./ClassGroupPicker";
import { t } from "../../i18n";
import {
  useScheduleQuery,
  type CourseDetail,
  type TaskView,
} from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";
import { SyllabusFrame } from "./syllabus-frame";

export type CourseTab = "overview" | "materials" | "assignments" | "syllabus";

/** Stored LMS item fields beyond the `Material` type. */
type LmsItem = Workspace["materials"][number] & {
  sectionNumber?: number;
  sectionTitle?: string;
  module?: string;
};

const MODULE_LABELS: Record<string, string> = {
  resource: "File",
  folder: "Folder",
  url: "Link",
  assign: "Assignment",
  quiz: "Quiz",
  forum: "Forum",
  page: "Page",
};

/** Opens in the app's Web page; the ↗ next to it opens the system browser. */
function OfficialLink({
  url,
  label,
  openWeb,
}: {
  url: string;
  label: React.ReactNode;
  openWeb: (url: string) => void;
}) {
  return (
    <span className="officialLink">
      <button type="button" onClick={() => openWeb(url)}>
        {label}
      </button>
      <button
        type="button"
        className="officialOutside"
        title={t("Open in browser")}
        aria-label={t("Open in browser")}
        onClick={() => void openExternalUrl(url).catch(() => undefined)}
      >
        ↗
      </button>
    </span>
  );
}

/**
 * A course's details, the same wherever it opens (Courses panel, Today,
 * Schedule): overview, LMS materials by section, assignments, syllabus.
 */
export function CourseDetails({
  course,
  data,
  initialTab = "overview",
  onTabChange,
  openWeb,
  fetchLms,
  progress = null,
}: {
  course: Course;
  data: Workspace;
  initialTab?: CourseTab;
  onTabChange?: (tab: CourseTab) => void;
  openWeb: (url: string) => void;
  /** Pulls LMS materials and assignments (the timetable is unchanged). */
  fetchLms?: () => void;
  /** 0–100 while an LMS pull runs, else null. */
  progress?: PageProgress;
}) {
  const [localTab, setTab] = useState<CourseTab>(initialTab);
  const tab = onTabChange ? initialTab : localTab;
  const now = useClock();
  const tasks = useScheduleQuery<TaskView>({ kind: "taskView", now }).value;
  const groupChoices = (
    useScheduleQuery<ClassGroupChoice[]>({
      kind: "classGroups",
      today: tokyoDate(new Date(now)),
    }).value ?? []
  ).filter((choice) => choice.courses.some((item) => item.id === course.id));
  const detail = useScheduleQuery<CourseDetail>({
    kind: "courseDetail",
    courseId: course.id,
  }).value;
  const rows = <T extends { id: string }>(items: T[], ids?: string[]): T[] => {
    const byId = new Map(items.map((item) => [item.id, item]));
    return (ids ?? []).flatMap((id) => byId.get(id) ?? []);
  };
  const materials = rows(data.materials as LmsItem[], detail?.materialIds);
  const sections = detail?.sections ?? [];
  const assignments = rows(data.assignments, detail?.assignmentIds);
  const offering = data.offerings.find(
    (item) => item.id === detail?.offeringId,
  );
  const exams = detail?.exams ?? [];
  // Plain text: a Badge shrinks to an empty pill when the tab runs out of room.
  const count = (value: number) =>
    value > 0 ? <span className="courseTabCount">{value}</span> : null;
  const refresh = fetchLms && (
    <Button
      size="compact-xs"
      variant="subtle"
      disabled={progress !== null}
      leftSection={
        progress !== null ? <SyncRing progress={progress} size={14} /> : undefined
      }
      onClick={fetchLms}
    >
      {progress !== null ? progressLabel(progress) : t("Fetch")}
    </Button>
  );

  return (
    <div className="courseDetails">
      <Tabs
        value={tab}
        onChange={(value) =>
          (onTabChange ?? setTab)((value ?? "overview") as CourseTab)
        }
      >
        <Tabs.List grow>
          <Tabs.Tab value="overview">{t("Overview")}</Tabs.Tab>
          <Tabs.Tab value="materials" rightSection={count(materials.length)}>
            {t("Materials")}
          </Tabs.Tab>
          <Tabs.Tab
            value="assignments"
            rightSection={count(assignments.length)}
          >
            {t("Assignments")}
          </Tabs.Tab>
          <Tabs.Tab value="syllabus">{t("Syllabus")}</Tabs.Tab>
        </Tabs.List>
      </Tabs>
      <ScrollArea className="courseDetailsBody" type="auto">
        {tab === "overview" && (
          <>
            <dl>
              <dt>{t("Schedule")}</dt>
              <dd>
                {course.meetings.length
                  ? course.meetings.map((meeting, index) => (
                      <div key={index}>
                        {meeting.days
                          .map((day) => t(dayNames[day - 1].slice(0, 3)))
                          .join(" / ")}{" "}
                        {meeting.startTime}–{meeting.endTime}
                        {meeting.room ? ` · ${meeting.room}` : ""}
                      </div>
                    ))
                  : t("集中・時間未定")}
              </dd>
              {groupChoices.length > 0 && (
                <>
                  <dt>{t("Physics group")}</dt>
                  <dd>
                    {groupChoices.map((choice) => (
                      <ClassGroupSwitch
                        key={choice.semester}
                        choice={choice}
                        onChange={(group) =>
                          void workspaceActions
                            .setClassGroup(choice.semester, group)
                            .catch(notify.error)
                        }
                      />
                    ))}
                  </dd>
                </>
              )}
              <dt>{t("Instructor")}</dt>
              <dd>{course.instructor || "—"}</dd>
              <dt>{t("Credits")}</dt>
              <dd>
                {course.creditSplit.lecture}-{course.creditSplit.exercise}-
                {course.creditSplit.lab} ({course.credits})
              </dd>
              {offering && (
                <>
                  <dt>{t("Format")}</dt>
                  <dd>{offering.format.kinds.join(" / ") || "—"}</dd>
                  <dt>{t("Offered by")}</dt>
                  <dd>{offering.kyomu?.department || "—"}</dd>
                </>
              )}
              {exams.length > 0 && (
                <>
                  <dt>{t("Exams")}</dt>
                  <dd className="courseExamList">
                    <ExamCards exams={exams} courses={[course]} />
                  </dd>
                </>
              )}
            </dl>
            <div className="courseDetailsLinks">
              {detail?.lmsUrl && (
                <OfficialLink
                  url={detail.lmsUrl}
                  label="LMS"
                  openWeb={openWeb}
                />
              )}
            </div>
          </>
        )}
        {tab === "materials" && (
          <div className="detailsList">
            <header>
              <span />
              {refresh}
            </header>
            {sections.map((section) => {
              const items = materials.filter(
                (item) => (item.sectionNumber ?? item.session) === section,
              );
              return (
                <section key={section}>
                  <h4>
                    {items[0]?.sectionTitle || `${t("Section")} ${section}`}
                  </h4>
                  {items.map((item) => (
                    <div className="detailsRow" key={item.id}>
                      {item.url ? (
                        <OfficialLink
                          url={item.url}
                          label={item.title}
                          openWeb={openWeb}
                        />
                      ) : (
                        <b>{item.title}</b>
                      )}
                      <small>
                        {t(MODULE_LABELS[item.module ?? ""] ?? item.type)}
                      </small>
                    </div>
                  ))}
                </section>
              );
            })}
            {!materials.length && (
              <div className="empty small">{t("No materials")}</div>
            )}
          </div>
        )}
        {tab === "assignments" && (
          <div className="detailsList">
            <header>
              <span />
              {refresh}
            </header>
            {assignments.map((assignment) => {
              const due = tasks?.deadlines?.[assignment.id];
              return (
                <div className="detailsRow" key={assignment.id}>
                  {assignment.sourceUrl ? (
                    <OfficialLink
                      url={assignment.sourceUrl}
                      label={assignment.title}
                      openWeb={openWeb}
                    />
                  ) : (
                    <b>{assignment.title}</b>
                  )}
                  <small
                    className={
                      due?.overdue && assignment.submission === "open"
                        ? "overdue"
                        : (due?.tone ?? "normal")
                    }
                  >
                    {due?.label ?? "—"} ·{" "}
                    {submissionLabel(
                      assignment,
                      t,
                      tasks?.submittedIds.includes(assignment.id) ?? false,
                    )}
                  </small>
                </div>
              );
            })}
            {!assignments.length && (
              <div className="empty small">{t("No assignments")}</div>
            )}
          </div>
        )}
        {tab === "syllabus" && <SyllabusFrame url={detail?.syllabusUrl} />}
      </ScrollArea>
    </div>
  );
}
