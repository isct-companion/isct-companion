import {
  Badge,
  Button,
  Drawer,
  Group,
  ScrollArea,
  Stack,
  Tabs,
  Text,
  ThemeIcon,
} from "@mantine/core";
import { useState } from "react";
import { deadlineState } from "../../core/academic";
import { t } from "../../i18n";
import { dayNames } from "../../components/timetable/timetable";
import type { Course, Workspace } from "../../core/model";

type DrawerTab = "overview" | "materials" | "assignments";

export function CourseDrawer({
  course,
  data,
  readOnly,
  onTimetable,
  initialTab = "overview",
  close,
  edit,
  toggle,
}: {
  course: Course;
  data: Workspace;
  readOnly: boolean;
  onTimetable: boolean;
  initialTab?: DrawerTab;
  close: () => void;
  edit: () => void;
  toggle: () => void;
}) {
  const [tab, setTab] = useState<DrawerTab>(initialTab);
  const materials = data.materials.filter(
    (item) => item.courseId === course.id && !item.deletedAt,
  );
  const assignments = data.assignments.filter(
    (item) => item.courseId === course.id && !item.deletedAt,
  );
  const count = (value: number) =>
    value > 0 ? <Badge size="xs">{value}</Badge> : null;

  return (
    <Drawer
      opened
      onClose={close}
      position="right"
      size="min(420px, 100%)"
      title={
        <Group gap="sm" wrap="nowrap">
          <ThemeIcon color={course.color} size={36} radius="md" aria-hidden />
          <div>
            <Text fw={700} lh={1.25}>
              {course.title}
            </Text>
            <Text c="dimmed" size="xs">
              {course.code} · {course.termId.replace("-", " ")}
            </Text>
          </div>
        </Group>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
      styles={{
        body: {
          height: "calc(100% - 64px)",
          padding: 0,
          display: "flex",
          flexDirection: "column",
        },
      }}
    >
      <Tabs
        value={tab}
        onChange={(value) => setTab((value ?? "overview") as DrawerTab)}
        className="courseDrawerTabs"
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
        </Tabs.List>
      </Tabs>

      <ScrollArea className="drawerBody" type="auto">
        {tab === "overview" ? (
          <dl>
            <dt>{t("Instructor")}</dt>
            <dd>{course.instructor || "—"}</dd>
            <dt>{t("Schedule")}</dt>
            <dd>
              {course.meetings.length
                ? course.meetings.map((meeting, index) => (
                    <div key={index}>
                      {meeting.days
                        .map((day) => dayNames[day - 1].slice(0, 3))
                        .join(" / ")} {" "}
                      {meeting.startTime}–{meeting.endTime}
                      {meeting.room ? ` · ${meeting.room}` : ""}
                    </div>
                  ))
                : "Not scheduled"}
            </dd>
            <dt>{t("Credits")}</dt>
            <dd>{course.credits}</dd>
            <dt>{t("Language")}</dt>
            <dd>{course.language || "—"}</dd>
          </dl>
        ) : tab === "materials" ? (
          <Stack className="drawerList" gap={0}>
            {materials.map((material) => (
              <article key={material.id}>
                <small>
                  Session {material.session} · {material.type}
                </small>
                {material.url ? (
                  <a href={material.url} target="_blank" rel="noreferrer">
                    {material.title} ↗
                  </a>
                ) : (
                  <b>{material.title}</b>
                )}
              </article>
            ))}
            {!materials.length && (
              <div className="empty small">No cached materials.</div>
            )}
          </Stack>
        ) : (
          <Stack className="drawerList" gap={0}>
            {assignments.map((assignment) => (
              <article key={assignment.id}>
                <small className={deadlineState(assignment).tone}>
                  {deadlineState(assignment).label}
                </small>
                {assignment.sourceUrl ? (
                  <a
                    href={assignment.sourceUrl}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {assignment.title} ↗
                  </a>
                ) : (
                  <b>{assignment.title}</b>
                )}
                {assignment.details && <p>{assignment.details}</p>}
              </article>
            ))}
            {!assignments.length && (
              <div className="empty small">No cached assignments.</div>
            )}
          </Stack>
        )}
      </ScrollArea>

      <Group className="drawerActions" justify="flex-end" gap="xs">
        {course.url && (
          <Button
            component="a"
            href={course.url}
            target="_blank"
            rel="noreferrer"
            variant="default"
          >
            Syllabus ↗
          </Button>
        )}
        {!readOnly && (
          <>
            <Button variant="default" onClick={toggle}>
              {onTimetable ? "Remove from timetable" : "Add to timetable"}
            </Button>
            <Button onClick={edit}>{t("Edit")}</Button>
          </>
        )}
      </Group>
    </Drawer>
  );
}
