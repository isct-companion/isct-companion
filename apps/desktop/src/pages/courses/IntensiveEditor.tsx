import {
  Button,
  Group,
  Modal,
  NumberInput,
  Stack,
  TextInput,
} from "../../components/ui";
import { useState } from "react";
import type { IntensiveSchedule } from "../../lib/model";
import { scheduleQuery } from "../../platform/core-query";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import { t } from "../../i18n";

export function IntensiveEditor({
  value,
  opened,
  close,
  save,
}: {
  value?: IntensiveSchedule;
  opened: boolean;
  close: () => void;
  save: (
    value: Pick<IntensiveSchedule, "from" | "to" | "periods" | "days" | "room">,
  ) => Promise<unknown> | void;
}) {
  const submission = useSubmissionGuard();
  const [from, setFrom] = useState(value?.from ?? "");
  const [to, setTo] = useState(value?.to ?? "");
  const [first, setFirst] = useState(value?.periods[0] ?? 1);
  const [last, setLast] = useState(value?.periods[1] ?? 6);
  const [days, setDays] = useState(value?.days ?? []);
  const [error, setError] = useState("");
  const quickSet = async () => {
    try {
      const value = await scheduleQuery<NonNullable<IntensiveSchedule["days"]>>(
        {
          kind: "intensiveDays",
          from,
          to,
          periods: [first, last],
        },
      );
      setDays(value);
      setError("");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : t("courses.invalidRange"),
      );
    }
  };
  return (
    <Modal
      opened={opened}
      onClose={() => !submission.busy && close()}
      closeOnClickOutside={!submission.busy}
      closeOnEscape={!submission.busy}
      title={t("courses.intensiveDates")}
      size="lg"
    >
      <Stack aria-busy={submission.busy}>
        <Group grow>
          <TextInput
            type="date"
            label={t("courses.from")}
            value={from}
            onChange={(e) => setFrom(e.currentTarget.value)}
          />
          <TextInput
            type="date"
            label={t("courses.to")}
            value={to}
            onChange={(e) => setTo(e.currentTarget.value)}
          />
        </Group>
        <Group grow>
          <NumberInput
            label={t("courses.firstPeriod")}
            min={1}
            max={10}
            value={first}
            onChange={(v) => setFirst(Number(v))}
          />
          <NumberInput
            label={t("courses.lastPeriod")}
            min={1}
            max={10}
            value={last}
            onChange={(v) => setLast(Number(v))}
          />
        </Group>
        <Button variant="default" onClick={() => void quickSet()}>
          {t("courses.setEveryDayInRange")}
        </Button>
        {error && <div role="alert">{error}</div>}
        {submission.error && <div role="alert">{submission.error}</div>}
        {days.map((day) => (
          <Group key={day.date} justify="space-between">
            <span>
              {day.date} ·{" "}
              {t("format.periodsRange", {
                range: day.periods?.join("–") ?? "",
              })}
            </span>
            <Button
              variant="subtle"
              color="red"
              onClick={() =>
                setDays((current) =>
                  current.filter((item) => item.date !== day.date),
                )
              }
            >
              {t("common.remove")}
            </Button>
          </Group>
        ))}
        <Group justify="flex-end">
          <Button variant="default" disabled={submission.busy} onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            disabled={submission.busy || !days.length}
            onClick={() =>
              void submission.run(() =>
                save({ from, to, periods: [first, last], days }),
              )
            }
          >
            {t("common.save")}
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
