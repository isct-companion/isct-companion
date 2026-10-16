import {
  Button,
  Group,
  Modal,
  SegmentedControl,
  Stack,
  Text,
} from "@mantine/core";
import { t } from "../../i18n";
import { classGroupLabel, semesterLabel } from "../../lib/alternation";
import type { ClassGroup, ClassGroupChoice } from "../../lib/model";

/** "10/16 is the Red group's exercise", to help recognise one's group. */
function nextHint(choice: ClassGroupChoice): string | undefined {
  const next = choice.next;
  if (!next || next.turn === "all") return undefined;
  return t(
    next.session === "lab"
      ? "{{date}} is the {{group}}'s lab ({{course}})."
      : "{{date}} is the {{group}}'s exercise ({{course}}).",
    {
      date: next.date.slice(5).replace("-", "/"),
      group: classGroupLabel(next.turn),
      course: next.courseTitle,
    },
  );
}

function unlisted(choice: ClassGroupChoice): boolean {
  return choice.courses.some((course) => !course.scheduled);
}

/** Red/white switch for one semester, shown in a physics course's details. */
export function ClassGroupSwitch({
  choice,
  onChange,
}: {
  choice: ClassGroupChoice;
  onChange: (group: ClassGroup) => void;
}) {
  return (
    <Stack gap={4}>
      <SegmentedControl
        size="xs"
        value={choice.group ?? ""}
        onChange={(value) => onChange(value as ClassGroup)}
        data={(["red", "white"] as const).map((group) => ({
          value: group,
          label: classGroupLabel(group),
        }))}
      />
      <Text size="xs" c="dimmed">
        {semesterLabel(choice.terms)}
        {unlisted(choice)
          ? ` · ${t("This class's dates are not listed yet, so it shows every week.")}`
          : ""}
      </Text>
    </Stack>
  );
}

/** Asked once after an LMS pull adds 物理学演習・実験 without a chosen group. */
export function ClassGroupPrompt({
  choice,
  choose,
  later,
}: {
  choice: ClassGroupChoice;
  choose: (group: ClassGroup) => void;
  later: () => void;
}) {
  const hint = nextHint(choice);
  return (
    <Modal
      opened
      onClose={later}
      title={`${t("Physics group")} · ${semesterLabel(choice.terms)}`}
    >
      <Stack gap="sm">
        <Text size="sm">
          {t(
            "物理学演習 and 物理学実験 meet every other week, in turns of a red and a white group. The timetable shows only your group's days.",
          )}
        </Text>
        <Text size="sm" fw={600}>
          {choice.courses.map((course) => course.title).join(" · ")}
        </Text>
        {hint && <Text size="sm">{hint}</Text>}
        {unlisted(choice) && (
          <Text size="xs" c="dimmed">
            {t(
              "This class's dates are not listed yet, so it shows every week.",
            )}
          </Text>
        )}
        <Text size="xs" c="dimmed">
          {t("You can change this later in the course details.")}
        </Text>
        <Group justify="flex-end" gap="xs">
          <Button variant="subtle" onClick={later}>
            {t("Later")}
          </Button>
          {(["red", "white"] as const).map((group) => (
            <Button key={group} onClick={() => choose(group)}>
              {classGroupLabel(group)}
            </Button>
          ))}
        </Group>
      </Stack>
    </Modal>
  );
}
