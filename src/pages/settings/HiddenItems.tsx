import { useState } from "react";
import { Checkbox } from "@mantine/core";
import { t } from "../../i18n";
import { UiButton } from "../../components/mantine";
import { pickLocalized, type Workspace } from "../../core/model";

export function HiddenItems({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const items = data.assignments
    .filter((assignment) => assignment.hidden && !assignment.deletedAt)
    .sort((a, b) => (a.dueAt ?? "").localeCompare(b.dueAt ?? ""));
  const courseTitle = (id: string) => {
    const course = data.courses.find((item) => item.id === id);
    return course
      ? pickLocalized(course.names?.title, data.settings.locale, course.title)
      : "";
  };
  const restore = (ids: Set<string> | null) => {
    update((draft) => {
      const now = new Date().toISOString();
      for (const assignment of draft.assignments) {
        if (!assignment.hidden || (ids && !ids.has(assignment.id))) continue;
        assignment.hidden = false;
        assignment.rev += 1;
        assignment.updatedAt = now;
      }
      return draft;
    });
    setSelected(new Set());
  };
  const toggle = (id: string, on: boolean) =>
    setSelected((prior) => {
      const next = new Set(prior);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  return (
    <>
      <div className="paneHead">
        <h2>{t("Hidden items")}</h2>
        {items.length > 0 && (
          <div className="actions">
            {selected.size > 0 && (
              <UiButton className="button" onClick={() => restore(selected)}>
                {t("Restore selected")} ({selected.size})
              </UiButton>
            )}
            <UiButton className="button" onClick={() => restore(null)}>
              {t("Restore all")}
            </UiButton>
          </div>
        )}
      </div>
      {items.length ? (
        <ul className="itemList">
          {items.map((assignment) => (
            <li key={assignment.id}>
              <Checkbox
                aria-label={assignment.title}
                checked={selected.has(assignment.id)}
                onChange={(event) =>
                  toggle(assignment.id, event.currentTarget.checked)
                }
              />
              <span className="itemMain">
                <b>{assignment.title}</b>
                <small>
                  {[
                    courseTitle(assignment.courseId),
                    assignment.dueAt
                      ? new Date(assignment.dueAt).toLocaleString(undefined, {
                          month: "short",
                          day: "numeric",
                          hour: "2-digit",
                          minute: "2-digit",
                        })
                      : t("No deadline"),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </small>
              </span>
              <UiButton
                className="button small"
                onClick={() => restore(new Set([assignment.id]))}
              >
                {t("Restore")}
              </UiButton>
            </li>
          ))}
        </ul>
      ) : (
        <p className="paneEmpty">{t("No hidden items")}</p>
      )}
    </>
  );
}
