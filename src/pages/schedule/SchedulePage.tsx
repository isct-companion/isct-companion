import { useMemo, useState } from "react";

import { detectConflicts } from "../../core/academic";
import type { PersonalEvent, Plan, TermId, Workspace } from "../../core/model";
import { Button } from "@mantine/core";
import {
  WeekTimetable,
  type Block,
} from "../../components/timetable/timetable";
import {
  timetableCourses,
  weekColumnsAt,
} from "../../components/timetable/workspace";

const weekLabel = (columns: ReturnType<typeof weekColumnsAt>) => {
  const first = new Date(`${columns[0].date}T12:00:00+09:00`);
  const last = new Date(`${columns[6].date}T12:00:00+09:00`);
  const format = new Intl.DateTimeFormat("en-GB", {
    day: "numeric",
    month: "short",
    timeZone: "Asia/Tokyo",
  });
  return `${format.format(first)} – ${format.format(last)}`;
};

export function Schedule({
  data,
  term,
  openCourse,
  openEvent,
  openPlan,
  newPlan,
}: {
  data: Workspace;
  term: TermId;
  openCourse: (courseId: string) => void;
  openEvent: (event: PersonalEvent) => void;
  openPlan: (plan: Plan) => void;
  newPlan: () => void;
}) {
  const [anchor, setAnchor] = useState(() => new Date());
  const courses = timetableCourses(data, term);
  const events = data.events.filter(
    (event) => event.termId === term && !event.deletedAt,
  );
  const conflicts = useMemo(
    () =>
      new Set(
        detectConflicts(courses, events).flatMap((conflict) => [
          conflict.leftId,
          conflict.rightId,
        ]),
      ),
    [courses, events],
  );
  const columns = weekColumnsAt(data, courses, events, anchor, conflicts);
  const move = (days: number) =>
    setAnchor((current) => {
      const next = new Date(current);
      next.setDate(next.getDate() + days);
      return next;
    });
  const select = (block: Block) => {
    if (block.kind === "course") openCourse(block.id);
    else if (block.kind === "event") {
      const event = events.find((item) => item.id === block.id);
      if (event) openEvent(event);
    } else {
      const plan = data.plans.find(
        (item) => item.id === block.id && !item.deletedAt,
      );
      if (plan) openPlan(plan);
    }
  };

  return (
    <div className="page schedulePage">
      <header className="pageBar scheduleHeader">
        <div>
          <h1>Schedule</h1>
          <span>{weekLabel(columns)}</span>
        </div>
        <div className="scheduleControls">
          <Button variant="default" onClick={newPlan}>
            ＋ Plan
          </Button>
          <UiButton
            type="button"
            aria-label="Previous week"
            onClick={() => move(-7)}
          >
            ←
          </UiButton>
          <UiButton type="button" onClick={() => setAnchor(new Date())}>
            This week
          </UiButton>
          <UiButton
            type="button"
            aria-label="Next week"
            onClick={() => move(7)}
          >
            →
          </UiButton>
        </div>
      </header>
      <section className="scheduleCanvas">
        <WeekTimetable
          days={columns}
          onSelect={select}
          showNow
          emptyLabel="Nothing scheduled this day"
        />
      </section>
    </div>
  );
}
import { UiButton } from "../../components/mantine";
