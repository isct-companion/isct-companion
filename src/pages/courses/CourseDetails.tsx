import { Badge, Button, ScrollArea, Tabs } from "@mantine/core";
import { useState } from "react";
import { openExternalUrl } from "../../components/ExternalLink";
import { dayNames } from "../../components/timetable/timetable";
import { deadlineState } from "../../core/academic";
import { pickLocalized, type Course, type Workspace } from "../../core/model";
import { t } from "../../i18n";
import { getDesktopBridge } from "../../platform/desktop-bridge";
import { SyllabusDetails } from "./syllabus-details";

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
  lmsUrl,
  openWeb,
  fetchLms,
  fetching,
}: {
  course: Course;
  data: Workspace;
  initialTab?: CourseTab;
  /** The course's LMS page, when it is linked to one. */
  lmsUrl?: string;
  openWeb: (url: string) => void;
  /** Pulls LMS materials and assignments (the timetable is unchanged). */
  fetchLms?: () => void;
  fetching?: boolean;
}) {
  const [tab, setTab] = useState<CourseTab>(initialTab);
  const bridge = getDesktopBridge();
  const locale = data.settings.locale;
  const materials = (data.materials as LmsItem[])
    .filter(
      (item) =>
        item.courseId === course.id &&
        !item.deletedAt &&
        item.module !== "assign" &&
        item.module !== "quiz",
    )
    .sort(
      (left, right) =>
        (left.sectionNumber ?? left.session) - (right.sectionNumber ?? right.session),
    );
  const sections = [...new Set(materials.map((item) => item.sectionNumber ?? item.session))];
  const assignments = data.assignments
    .filter((item) => item.courseId === course.id && !item.deletedAt && !item.hidden)
    .sort((left, right) => (left.dueAt ?? "~").localeCompare(right.dueAt ?? "~"));
  const offering = data.offerings.find(
    (item) => item.id === course.offeringId && !item.deletedAt,
  );
  const count = (value: number) => (value > 0 ? <Badge size="xs">{value}</Badge> : null);
  const refresh = fetchLms && (
    <Button size="compact-xs" variant="subtle" loading={fetching} onClick={fetchLms}>
      {t("Fetch")}
    </Button>
  );

  return (
    <div className="courseDetails">
      <Tabs value={tab} onChange={(value) => setTab((value ?? "overview") as CourseTab)}>
        <Tabs.List grow>
          <Tabs.Tab value="overview">{t("Overview")}</Tabs.Tab>
          <Tabs.Tab value="materials" rightSection={count(materials.length)}>
            {t("Materials")}
          </Tabs.Tab>
          <Tabs.Tab value="assignments" rightSection={count(assignments.length)}>
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
                        {meeting.days.map((day) => t(dayNames[day - 1].slice(0, 3))).join(" / ")}{" "}
                        {meeting.startTime}–{meeting.endTime}
                        {meeting.room ? ` · ${meeting.room}` : ""}
                      </div>
                    ))
                  : t("集中・時間未定")}
              </dd>
              <dt>{t("Instructor")}</dt>
              <dd>{course.instructor || "—"}</dd>
              <dt>{t("Credits")}</dt>
              <dd>
                {course.creditSplit.lecture}-{course.creditSplit.exercise}-{course.creditSplit.lab} (
                {course.credits})
              </dd>
              {offering && (
                <>
                  <dt>{t("Format")}</dt>
                  <dd>
                    {offering.format.kinds.join(" / ") || "—"}
                    {offering.format.delivery ? ` · ${offering.format.delivery}` : ""}
                  </dd>
                  <dt>{t("Academic unit")}</dt>
                  <dd>{pickLocalized(offering.academicUnit, locale, "—")}</dd>
                </>
              )}
            </dl>
            <div className="courseDetailsLinks">
              {lmsUrl && <OfficialLink url={lmsUrl} label="LMS" openWeb={openWeb} />}
              {course.url && <OfficialLink url={course.url} label={t("Syllabus")} openWeb={openWeb} />}
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
              const items = materials.filter((item) => (item.sectionNumber ?? item.session) === section);
              return (
                <section key={section}>
                  <h4>{items[0]?.sectionTitle || `${t("Section")} ${section}`}</h4>
                  {items.map((item) => (
                    <div className="detailsRow" key={item.id}>
                      {item.url ? (
                        <OfficialLink url={item.url} label={item.title} openWeb={openWeb} />
                      ) : (
                        <b>{item.title}</b>
                      )}
                      <small>{t(MODULE_LABELS[item.module ?? ""] ?? item.type)}</small>
                    </div>
                  ))}
                </section>
              );
            })}
            {!materials.length && <div className="empty small">{t("No materials")}</div>}
          </div>
        )}
        {tab === "assignments" && (
          <div className="detailsList">
            <header>
              <span />
              {refresh}
            </header>
            {assignments.map((assignment) => {
              const due = deadlineState(assignment);
              return (
                <div className="detailsRow" key={assignment.id}>
                  {assignment.sourceUrl ? (
                    <OfficialLink url={assignment.sourceUrl} label={assignment.title} openWeb={openWeb} />
                  ) : (
                    <b>{assignment.title}</b>
                  )}
                  <small className={due.ms < 0 && assignment.submission === "open" ? "overdue" : due.tone}>
                    {assignment.dueAt ? due.label : "—"} · {t(assignment.submission ?? "unknown")}
                  </small>
                </div>
              );
            })}
            {!assignments.length && <div className="empty small">{t("No assignments")}</div>}
          </div>
        )}
        {tab === "syllabus" &&
          (course.url ? (
            <SyllabusDetails
              url={course.url}
              fetchCourse={
                bridge
                  ? (url) => bridge.syllabusCourse(url, locale === "ja" ? "ja" : "en")
                  : undefined
              }
            />
          ) : (
            <div className="empty small">{t("No syllabus is linked to this course.")}</div>
          ))}
      </ScrollArea>
    </div>
  );
}
