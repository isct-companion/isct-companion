import { useState } from "react";
import { Paper, Input } from "@mantine/core";
import { UiButton } from "../../components/mantine";
import {
  WeekTimetable,
  type Block,
} from "../../components/timetable/timetable";
import { agendaColumns } from "../../components/timetable/workspace";
import { useScheduleQuery, type ScheduleView } from "../../platform/core-query";
import {
  addCivilDays,
  civilWeekday,
  formatCivilDate,
  tokyoDate,
} from "../../lib/civil-date";
import { resolvedTeachingPeriods } from "../../lib/timetable-layout";
import type { Workspace } from "../../lib/model";
import { t } from "../../i18n";

/** Date navigation is independent of the quarter used for course planning. */
export function CalendarView({
  data,
  open,
}: {
  data: Workspace;
  open: (block: Block) => void;
}) {
  const today = tokyoDate(new Date());
  const [anchor, setAnchor] = useState<string>();
  const date = anchor ?? today;
  const from = addCivilDays(date, 1 - civilWeekday(date));
  const to = addCivilDays(from, 6);
  const view = useScheduleQuery<ScheduleView>({
    kind: "scheduleView",
    term: data.currentTerm,
    from,
    to,
    includePersonal: true,
  });
  const visibleFrom = view.value?.agendas[0]?.date ?? from;
  const visibleTo = view.value?.agendas.at(-1)?.date ?? to;
  return (
    <section className="calendarWorkspace" aria-label={t("Calendar")}>
      <header className="scheduleTitleRow">
        <h1>{t("Calendar")}</h1>
      </header>
      <Paper className="ttCard" withBorder radius="md">
        <header className="timetableViewBar">
          <b>
            {formatCivilDate(visibleFrom, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}{" "}
            –{" "}
            {formatCivilDate(visibleTo, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}
          </b>
          <div className="scheduleControls">
            <Input
              type="date"
              aria-label={t("Date")}
              value={date}
              onChange={(event) =>
                event.target.value && setAnchor(event.target.value)
              }
            />
            <UiButton
              aria-label={t("Previous week")}
              onClick={() => setAnchor(addCivilDays(date, -7))}
            >
              ←
            </UiButton>
            <UiButton onClick={() => setAnchor(undefined)}>
              {t("This week")}
            </UiButton>
            <UiButton
              aria-label={t("Next week")}
              onClick={() => setAnchor(addCivilDays(date, 7))}
            >
              →
            </UiButton>
          </div>
        </header>
        <WeekTimetable
          days={agendaColumns(view.value?.agendas ?? [])}
          teachingPeriods={resolvedTeachingPeriods(
            data.academicProfile.teachingPeriods,
          )}
          onSelect={open}
          showNow
          emptyLabel="Nothing scheduled this day"
        />
      </Paper>
    </section>
  );
}
