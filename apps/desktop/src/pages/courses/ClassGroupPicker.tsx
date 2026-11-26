import { SegmentedControl, Stack, Text } from "../../components/ui";
import type { NoticeDialog } from "../../components/confirm";
import { t } from "../../i18n";
import { classGroupLabel, semesterLabel } from "../../lib/alternation";
import type { ClassGroup, ClassGroupChoice } from "../../lib/model";

/** "10/16 is the Red group's exercise", to help recognise one's group. */
function nextHint(choice: ClassGroupChoice): string | undefined {
  const next = choice.next;
  if (!next || next.turn === "all") return undefined;
  return t(
    next.session === "lab"
      ? "courses.dateIsTheGroupSLab"
      : "courses.dateIsTheGroupS",
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
    <Stack gap="space-2">
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
        {unlisted(choice) ? ` · ${t("courses.thisClasssDatesAreNot")}` : ""}
      </Text>
    </Stack>
  );
}

/** Asked once after an LMS pull adds 物理学演習・実験 without a chosen group. */
export function classGroupDialog(
  choice: ClassGroupChoice,
  choose: (group: ClassGroup) => void,
  later: () => void,
): NoticeDialog {
  const hint = nextHint(choice);
  return {
    id: `physics:${choice.semester}`,
    title: `${t("courses.physicsGroup")} · ${semesterLabel(choice.terms)}`,
    message: (
      <Stack gap="space-5">
        <Text size="sm">{t("courses.andMeetEveryOtherWeek")}</Text>
        <Text size="sm" fw={600}>
          {choice.courses.map((course) => course.title).join(" · ")}
        </Text>
        {hint && <Text size="sm">{hint}</Text>}
        {unlisted(choice) && (
          <Text size="xs" c="dimmed">
            {t("courses.thisClasssDatesAreNot")}
          </Text>
        )}
        <Text size="xs" c="dimmed">
          {t("courses.youCanChangeThisLater")}
        </Text>
      </Stack>
    ),
    choices: [
      { id: "later", label: t("common.later"), variant: "subtle" },
      ...(["red", "white"] as const).map((group) => ({
        id: group,
        label: classGroupLabel(group),
        variant: "light" as const,
      })),
    ],
    onChoice: (picked) => {
      if (picked === "red" || picked === "white") choose(picked);
      else later();
    },
  };
}
