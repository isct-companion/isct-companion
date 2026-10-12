import { Button, Drawer, Group, Text, ThemeIcon } from "@mantine/core";
import { t } from "../../i18n";
import type { Course, Workspace } from "../../core/model";
import { CourseDetails, type CourseTab } from "./CourseDetails";

/** A course opened from Today or Schedule; the same details as the Courses panel. */
export function CourseDrawer({
  course,
  data,
  readOnly,
  onTimetable,
  initialTab = "overview",
  onTabChange,
  close,
  edit,
  toggle,
  editIntensive,
  lmsUrl,
  openWeb,
  fetchLms,
  progress,
}: {
  course: Course;
  data: Workspace;
  readOnly: boolean;
  onTimetable: boolean;
  initialTab?: CourseTab;
  onTabChange?: (tab: CourseTab) => void;
  close: () => void;
  edit: () => void;
  toggle: () => void;
  editIntensive: () => void;
  lmsUrl?: string;
  openWeb: (url: string) => void;
  fetchLms?: () => void;
  progress?: number | null;
}) {
  return (
    <Drawer
      opened
      onClose={close}
      position="right"
      size="min(440px, 100%)"
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
      <CourseDetails
        key={course.id}
        course={course}
        data={data}
        initialTab={initialTab}
        onTabChange={onTabChange}
        lmsUrl={lmsUrl}
        openWeb={openWeb}
        fetchLms={fetchLms}
        progress={progress}
      />
      {!readOnly && (
        <Group className="drawerActions" justify="flex-end" gap="xs">
          <Button variant="default" onClick={editIntensive}>
            {t("Intensive dates")}
          </Button>
          <Button variant="default" onClick={toggle}>
            {onTimetable ? t("Remove from timetable") : t("Add to timetable")}
          </Button>
          <Button onClick={edit}>{t("Edit")}</Button>
        </Group>
      )}
    </Drawer>
  );
}
