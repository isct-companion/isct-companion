import { Button, Group, Modal, NumberInput, Stack, TextInput } from "@mantine/core";
import { useState } from "react";
import type { IntensiveSchedule } from "../../core/model";
import { intensiveDays } from "../../core/user-schedule";

export function IntensiveEditor({ value, opened, close, save }: {
  value?: IntensiveSchedule;
  opened: boolean;
  close: () => void;
  save: (value: Pick<IntensiveSchedule, "from" | "to" | "periods" | "days" | "room">) => void;
}) {
  const [from, setFrom] = useState(value?.from ?? "");
  const [to, setTo] = useState(value?.to ?? "");
  const [first, setFirst] = useState(value?.periods[0] ?? 1);
  const [last, setLast] = useState(value?.periods[1] ?? 6);
  const [days, setDays] = useState(value?.days ?? []);
  const [error, setError] = useState("");
  const quickSet = () => {
    try { setDays(intensiveDays(from, to, [first, last])); setError(""); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Invalid range"); }
  };
  return <Modal opened={opened} onClose={close} title="Intensive dates" size="lg"><Stack>
    <Group grow><TextInput type="date" label="From" value={from} onChange={(e) => setFrom(e.currentTarget.value)} /><TextInput type="date" label="To" value={to} onChange={(e) => setTo(e.currentTarget.value)} /></Group>
    <Group grow><NumberInput label="First period" min={1} max={10} value={first} onChange={(v) => setFirst(Number(v))} /><NumberInput label="Last period" min={1} max={10} value={last} onChange={(v) => setLast(Number(v))} /></Group>
    <Button variant="default" onClick={quickSet}>Set every day in range</Button>
    {error && <div role="alert">{error}</div>}
    {days.map((day) => <Group key={day.date} justify="space-between"><span>{day.date} · periods {day.periods?.join("–")}</span><Button variant="subtle" color="red" onClick={() => setDays((current) => current.filter((item) => item.date !== day.date))}>Remove</Button></Group>)}
    <Group justify="flex-end"><Button variant="default" onClick={close}>Cancel</Button><Button disabled={!days.length} onClick={() => save({ from, to, periods: [first, last], days })}>Save</Button></Group>
  </Stack></Modal>;
}
