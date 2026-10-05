import { Button, Group, Modal, Select, Stack, TextInput } from "@mantine/core";
import { useMemo, useState } from "react";
import type { Plan } from "../../core/model";
import { useEffect } from "react";
import { nextPlanOccurrences, planCivilStart } from "../../core/user-schedule";

const presets = {
  once: "",
  daily: "FREQ=DAILY",
  weekdays: "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
  weekly: "FREQ=WEEKLY",
  fortnightly: "FREQ=WEEKLY;INTERVAL=2",
};

export type PlanDraft = Pick<
  Plan,
  | "title"
  | "courseId"
  | "room"
  | "start"
  | "duration"
  | "rrule"
  | "exdates"
  | "reminder"
>;

export function PlanEditor({
  plan,
  opened,
  close,
  save,
  remove,
}: {
  plan?: Plan;
  opened: boolean;
  close: () => void;
  save: (draft: PlanDraft) => void;
  remove?: () => void;
}) {
  const [title, setTitle] = useState(plan?.title ?? "");
  const [start, setStart] = useState(
    plan
      ? `${planCivilStart(plan).date}T${planCivilStart(plan).time}`
      : new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(
          new Date(),
        ) + "T09:00",
  );
  const [duration, setDuration] = useState(plan?.duration ?? 60);
  const [room, setRoom] = useState(plan?.room ?? "");
  const [rule, setRule] = useState(plan?.rrule ?? "");
  const localPreview = () =>
    nextPlanOccurrences(
      {
        id: plan?.id ?? "preview",
        rev: plan?.rev ?? 1,
        updatedAt: plan?.updatedAt ?? new Date().toISOString(),
        updatedBy: plan?.updatedBy ?? "preview",
        title: title || "Preview",
        start,
        duration,
        rrule: rule || undefined,
        exdates: plan?.exdates ?? [],
      },
      5,
    );
  const [preview, setPreview] = useState<string[]>([]);
  const [ruleError, setRuleError] = useState("");
  const preset = useMemo(
    () =>
      Object.entries(presets).find(([, value]) => value === rule)?.[0] ??
      "advanced",
    [rule],
  );
  useEffect(() => {
    try {
      setPreview(start ? localPreview() : []);
      setRuleError("");
    } catch (error) {
      setPreview([]);
      setRuleError(error instanceof Error ? error.message : String(error));
    }
  }, [rule, start, plan?.exdates]);
  return (
    <Modal
      opened={opened}
      onClose={close}
      title={plan ? "Edit plan" : "New plan"}
    >
      <Stack>
        <TextInput
          required
          label="Title"
          value={title}
          onChange={(event) => setTitle(event.currentTarget.value)}
        />
        <TextInput
          required
          type="datetime-local"
          label="Start"
          value={start}
          onChange={(event) => setStart(event.currentTarget.value)}
        />
        <TextInput
          type="number"
          min={1}
          label="Duration (minutes)"
          value={duration}
          onChange={(event) => setDuration(Number(event.currentTarget.value))}
        />
        <TextInput
          label="Room"
          value={room}
          onChange={(event) => setRoom(event.currentTarget.value)}
        />
        <Select
          label="Repeats"
          value={preset}
          data={[
            { value: "once", label: "One-off" },
            { value: "daily", label: "Every day" },
            { value: "weekdays", label: "Weekdays" },
            { value: "weekly", label: "Weekly" },
            { value: "fortnightly", label: "Every 2 weeks" },
            { value: "advanced", label: "Advanced RRULE" },
          ]}
          onChange={(value) =>
            value &&
            value !== "advanced" &&
            setRule(presets[value as keyof typeof presets])
          }
        />
        {(preset === "advanced" || rule) && (
          <TextInput
            label="RRULE"
            description="DAILY, WEEKLY, MONTHLY or YEARLY; minimum one day"
            value={rule}
            onChange={(event) =>
              setRule(event.currentTarget.value.replace(/^RRULE:/i, ""))
            }
          />
        )}
        {ruleError && <div role="alert">{ruleError}</div>}
        {preview.length > 0 && (
          <div>
            <b>Next 5 occurrences</b>
            {preview.map((value) => (
              <div key={value}>{value}</div>
            ))}
          </div>
        )}
        <Group justify="space-between">
          {remove ? (
            <Button color="red" variant="subtle" onClick={remove}>
              Delete
            </Button>
          ) : (
            <span />
          )}
          <Group>
            <Button variant="default" onClick={close}>
              Cancel
            </Button>
            <Button
              disabled={
                !title.trim() ||
                !start ||
                !Number.isFinite(duration) ||
                duration < 1 ||
                Boolean(ruleError)
              }
              onClick={() =>
                save({
                  title: title.trim(),
                  start,
                  duration,
                  room: room.trim() || undefined,
                  rrule: rule || undefined,
                  exdates: plan?.exdates ?? [],
                  reminder: plan?.reminder,
                })
              }
            >
              Save
            </Button>
          </Group>
        </Group>
      </Stack>
    </Modal>
  );
}
