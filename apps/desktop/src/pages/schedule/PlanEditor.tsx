import { Button, Group, Modal, Select, Stack, TextInput } from "@mantine/core";
import { useMemo, useRef, useState } from "react";
import type { Plan } from "../../lib/model";
import { useEffect } from "react";
import {
  scheduleQuery,
  useScheduleQuery,
  type PlanCivilStart,
} from "../../platform/core-query";
import { useSubmissionGuard } from "../../components/use-submission-guard";

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
  save: (draft: PlanDraft) => Promise<unknown> | void;
  remove?: () => Promise<unknown> | void;
}) {
  const submission = useSubmissionGuard();
  const [title, setTitle] = useState(plan?.title ?? "");
  const [defaultStart] = useState(
    () => plan?.start ?? new Date().toISOString(),
  );
  const civilStart = useScheduleQuery<PlanCivilStart>({
    kind: "planCivilStart",
    plan: { start: defaultStart },
  }).value;
  const [start, setStart] = useState("");
  const editedStart = useRef(false);
  useEffect(() => {
    if (civilStart && !editedStart.current)
      setStart(`${civilStart.date}T${plan ? civilStart.time : "09:00"}`);
  }, [civilStart, plan]);
  const [duration, setDuration] = useState(plan?.duration ?? 60);
  const [room, setRoom] = useState(plan?.room ?? "");
  const [rule, setRule] = useState(plan?.rrule ?? "");
  const previewPlan = () => ({
    id: plan?.id ?? "preview",
    rev: plan?.rev ?? 1,
    updatedAt: plan?.updatedAt ?? new Date().toISOString(),
    updatedBy: plan?.updatedBy ?? "preview",
    title: title || "Preview",
    start,
    duration,
    rrule: rule || undefined,
    exdates: plan?.exdates ?? [],
  });
  const [preview, setPreview] = useState<string[]>([]);
  const [ruleError, setRuleError] = useState("");
  const preset = useMemo(
    () =>
      Object.entries(presets).find(([, value]) => value === rule)?.[0] ??
      "advanced",
    [rule],
  );
  useEffect(() => {
    let active = true;
    if (!start) {
      setPreview([]);
      setRuleError("");
      return;
    }
    void scheduleQuery<string[]>({
      kind: "nextPlanOccurrences",
      plan: previewPlan(),
      limit: 5,
    }).then(
      (value) => {
        if (!active) return;
        setPreview(value);
        setRuleError("");
      },
      (error) => {
        if (!active) return;
        setPreview([]);
        setRuleError(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      active = false;
    };
  }, [rule, start, plan?.exdates]);
  return (
    <Modal
      opened={opened}
      onClose={() => !submission.busy && close()}
      closeOnClickOutside={!submission.busy}
      closeOnEscape={!submission.busy}
      title={plan ? "Edit plan" : "New plan"}
    >
      <Stack aria-busy={submission.busy}>
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
          onChange={(event) => {
            editedStart.current = true;
            setStart(event.currentTarget.value);
          }}
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
        {submission.error && <div role="alert">{submission.error}</div>}
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
            <Button
              color="red"
              variant="subtle"
              disabled={submission.busy}
              onClick={() => void submission.run(remove)}
            >
              Delete
            </Button>
          ) : (
            <span />
          )}
          <Group>
            <Button
              variant="default"
              disabled={submission.busy}
              onClick={close}
            >
              Cancel
            </Button>
            <Button
              disabled={
                submission.busy ||
                !title.trim() ||
                !start ||
                !Number.isFinite(duration) ||
                duration < 1 ||
                Boolean(ruleError)
              }
              onClick={() =>
                void submission.run(() =>
                  save({
                    title: title.trim(),
                    start,
                    duration,
                    room: room.trim() || undefined,
                    rrule: rule || undefined,
                    exdates: plan?.exdates ?? [],
                    reminder: plan?.reminder,
                  }),
                )
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
