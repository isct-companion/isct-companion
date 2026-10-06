import { useMemo, useState, type FormEvent } from "react";
import type { PersonalEvent, TermId, Workspace } from "../../core/model";
import { newUserRecordId } from "../../storage/ids";
import "./events.css";

type Props = {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  notify: (message: string) => void;
  readOnly: boolean;
};

type Draft = Pick<
  PersonalEvent,
  | "title"
  | "room"
  | "termId"
  | "date"
  | "days"
  | "startTime"
  | "endTime"
  | "blocking"
>;

type Locale = Workspace["settings"]["locale"];
const tr = (locale: Locale, en: string, ja: string, zh: string) =>
  locale === "ja" ? ja : locale === "zh" ? zh : en;
const weekdayNames = (locale: Locale) =>
  locale === "ja"
    ? ["月曜日", "火曜日", "水曜日", "木曜日", "金曜日"]
    : locale === "zh"
      ? ["星期一", "星期二", "星期三", "星期四", "星期五"]
      : ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
const emptyDraft = (termId: TermId): Draft => ({
  title: "",
  room: "",
  termId,
  days: [1],
  startTime: "13:30",
  endTime: "15:10",
  blocking: true,
});
const overlaps = (a: Draft, b: PersonalEvent) =>
  a.startTime < b.endTime && b.startTime < a.endTime;

export function EventManager({ data, update, notify, readOnly }: Props) {
  const locale = data.settings.locale;
  const weekdays = weekdayNames(locale);
  const languageTag = locale === "zh" ? "zh-CN" : locale;
  const formatDate = (date: string) =>
    new Intl.DateTimeFormat(languageTag, {
      month: "short",
      day: "numeric",
      weekday: "short",
      timeZone: "Asia/Tokyo",
    }).format(new Date(`${date}T12:00:00+09:00`));
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const [draft, setDraft] = useState<Draft>(() =>
    emptyDraft(data.selectedTerm),
  );
  const [lastDeleted, setLastDeleted] = useState<string | null>(null);
  const [conflictWarning, setConflictWarning] = useState<string | null>(null);
  const events = useMemo(
    () =>
      data.events
        .filter((event) => !event.deletedAt)
        .sort((a, b) =>
          `${a.termId}${a.date || ""}${a.startTime}`.localeCompare(
            `${b.termId}${b.date || ""}${b.startTime}`,
          ),
        ),
    [data.events],
  );

  const beginNew = () => {
    if (readOnly) return;
    setDraft(emptyDraft(data.selectedTerm));
    setConflictWarning(null);
    setEditing("new");
  };
  const beginEdit = (event: PersonalEvent) => {
    if (readOnly) return;
    setDraft({
      title: event.title,
      room: event.room,
      termId: event.termId,
      date: event.date,
      days: event.days,
      startTime: event.startTime,
      endTime: event.endTime,
      blocking: event.blocking,
    });
    setConflictWarning(null);
    setEditing(event.id);
  };
  const conflict = (candidate: Draft) => {
    if (!candidate.blocking) return undefined;
    const sameSchedule = (event: PersonalEvent) => {
      if (
        event.id === editing ||
        event.deletedAt ||
        !event.blocking ||
        event.termId !== candidate.termId ||
        !overlaps(candidate, event)
      )
        return false;
      if (candidate.date || event.date)
        return Boolean(
          candidate.date && event.date && candidate.date === event.date,
        );
      return Boolean(candidate.days?.some((day) => event.days?.includes(day)));
    };
    const event = data.events.find(sameSchedule);
    if (event) return event.title;
    const candidateDays = candidate.date
      ? [new Date(`${candidate.date}T12:00:00+09:00`).getUTCDay()]
      : candidate.days;
    const course = data.courses.find(
      (item) =>
        !item.deletedAt &&
        item.termId === candidate.termId &&
        item.meetings.some(
          (meeting) =>
            candidateDays?.some((day) => meeting.days.includes(day)) &&
            candidate.startTime < meeting.endTime &&
            meeting.startTime < candidate.endTime,
        ),
    );
    return course?.overrides?.title || course?.title;
  };
  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (readOnly) return;
    if (
      !draft.title.trim() ||
      draft.startTime >= draft.endTime ||
      (!draft.date && !draft.days?.length)
    ) {
      notify(
        tr(
          locale,
          "Check the title, schedule, and time range",
          "タイトル、日程、時間帯を確認してください",
          "请检查标题、日程和时间范围",
        ),
      );
      return;
    }
    const blockedBy = conflict(draft);
    if (blockedBy && conflictWarning !== blockedBy) {
      setConflictWarning(blockedBy);
      notify(
        tr(
          locale,
          `Conflict with ${blockedBy}. Review it, then save again to continue.`,
          `${blockedBy} と重複しています。確認後、もう一度保存してください。`,
          `与 ${blockedBy} 冲突。确认后再次保存即可继续。`,
        ),
      );
      return;
    }
    const now = new Date().toISOString();
    const eventId = editing === "new" ? await newUserRecordId() : "";
    update((workspace) => ({
      ...workspace,
      events:
        editing === "new"
          ? [
              ...workspace.events,
              {
                ...draft,
                id: eventId,
                kind: "event",
                rev: 1,
                updatedAt: now,
                updatedBy: workspace.deviceId,
              },
            ]
          : workspace.events.map((item) =>
              item.id === editing
                ? {
                    ...item,
                    ...draft,
                    rev: item.rev + 1,
                    updatedAt: now,
                    updatedBy: workspace.deviceId,
                  }
                : item,
            ),
    }));
    notify(
      editing === "new"
        ? tr(
            locale,
            "Personal event saved",
            "個人予定を保存しました",
            "个人日程已保存",
          )
        : tr(
            locale,
            "Personal event updated",
            "個人予定を更新しました",
            "个人日程已更新",
          ),
    );
    setConflictWarning(null);
    setEditing(null);
  };
  const remove = (id: string) => {
    if (readOnly) return;
    const now = new Date().toISOString();
    update((workspace) => ({
      ...workspace,
      events: workspace.events.map((item) =>
        item.id === id
          ? {
              ...item,
              deletedAt: now,
              updatedAt: now,
              updatedBy: workspace.deviceId,
              rev: item.rev + 1,
            }
          : item,
      ),
    }));
    setLastDeleted(id);
    setEditing(null);
    notify(
      tr(
        locale,
        "Event deleted — Undo is available below",
        "予定を削除しました。下から元に戻せます",
        "日程已删除，可在下方撤销",
      ),
    );
  };
  const undo = () => {
    if (readOnly || !lastDeleted) return;
    const now = new Date().toISOString();
    update((workspace) => ({
      ...workspace,
      events: workspace.events.map((item) =>
        item.id === lastDeleted
          ? {
              ...item,
              deletedAt: undefined,
              updatedAt: now,
              updatedBy: workspace.deviceId,
              rev: item.rev + 1,
            }
          : item,
      ),
    }));
    setLastDeleted(null);
    notify(tr(locale, "Event restored", "予定を元に戻しました", "日程已恢复"));
  };

  return (
    <section className="surface featureManager" aria-labelledby="events-title">
      <div className="surfaceHead featureHead">
        <div>
          <h2 id="events-title">
            {tr(locale, "Personal events", "個人予定", "个人日程")}
          </h2>
          <span>
            {tr(
              locale,
              "One-off dates or a weekly quarter schedule",
              "単発の日付またはクォーター内の毎週予定",
              "单次日期或季度内每周日程",
            )}
          </span>
        </div>
        {!readOnly && (
          <UiButton className="button primary" type="button" onClick={beginNew}>
            ＋ {tr(locale, "Add event", "予定を追加", "添加日程")}
          </UiButton>
        )}
      </div>
      {events.length ? (
        <div className="eventList">
          {events.map((event) => (
            <article className="eventRow" key={event.id}>
              <time>
                {event.startTime}
                <small>{event.endTime}</small>
              </time>
              <div>
                <strong>{event.title}</strong>
                <small>
                  {(event.date && formatDate(event.date)) ||
                    event.days?.map((day) => weekdays[day - 1]).join(", ")}{" "}
                  · {event.termId}
                  {event.room ? ` · ${event.room}` : ""}
                </small>
              </div>
              <span className={event.blocking ? "conflictOn" : "conflictOff"}>
                {event.blocking
                  ? tr(locale, "Checks conflicts", "重複を確認", "检查冲突")
                  : tr(locale, "Allows overlap", "重複を許可", "允许重叠")}
              </span>
              {!readOnly && (
                <div className="rowActions">
                  <UiButton type="button" onClick={() => beginEdit(event)}>
                    {tr(locale, "Edit", "編集", "编辑")}
                  </UiButton>
                  <UiButton
                    className="dangerText"
                    type="button"
                    onClick={() => remove(event.id)}
                  >
                    {tr(locale, "Delete", "削除", "删除")}
                  </UiButton>
                </div>
              )}
            </article>
          ))}
        </div>
      ) : (
        <p className="emptyState">
          {tr(
            locale,
            "No personal events yet.",
            "個人予定はまだありません。",
            "暂无个人日程。",
          )}
        </p>
      )}
      {!readOnly && lastDeleted && (
        <div className="undoBar">
          <span>
            {tr(locale, "Event deleted", "予定を削除しました", "日程已删除")}
          </span>
          <UiButton type="button" onClick={undo}>
            {tr(locale, "Undo", "元に戻す", "撤销")}
          </UiButton>
        </div>
      )}
      {editing && !readOnly && (
        <form className="featureForm" onSubmit={save}>
          <div className="formTitle">
            <strong>
              {editing === "new"
                ? tr(locale, "New event", "新しい予定", "新建日程")
                : tr(locale, "Edit event", "予定を編集", "编辑日程")}
            </strong>
            <UiButton
              type="button"
              onClick={() => setEditing(null)}
              aria-label={tr(
                locale,
                "Close editor",
                "編集画面を閉じる",
                "关闭编辑器",
              )}
            >
              ×
            </UiButton>
          </div>
          <label className="wide">
            {tr(locale, "Title", "タイトル", "标题")}
            <UiInput
              required
              value={draft.title}
              onChange={(e) => setDraft({ ...draft, title: e.target.value })}
            />
          </label>
          <label>
            {tr(locale, "Quarter", "クォーター", "季度")}
            <UiInput
              required
              pattern="[0-9]{4}-[1-4]Q"
              value={draft.termId}
              onChange={(e) =>
                setDraft({ ...draft, termId: e.target.value as TermId })
              }
            />
          </label>
          <label>
            {tr(locale, "Room", "教室", "教室")}
            <UiInput
              value={draft.room}
              onChange={(e) => setDraft({ ...draft, room: e.target.value })}
            />
          </label>
          <fieldset className="wide">
            <legend>{tr(locale, "Schedule", "日程", "日程")}</legend>
            <label className="inlineChoice">
              <UiInput
                type="radio"
                checked={!draft.date}
                onChange={() =>
                  setDraft({
                    ...draft,
                    date: undefined,
                    days: draft.days?.length ? draft.days : [1],
                  })
                }
              />{" "}
              {tr(locale, "Weekly", "毎週", "每周")}
            </label>
            <label className="inlineChoice">
              <UiInput
                type="radio"
                checked={Boolean(draft.date)}
                onChange={() =>
                  setDraft({
                    ...draft,
                    date: new Date().toISOString().slice(0, 10),
                    days: undefined,
                  })
                }
              />{" "}
              {tr(locale, "One date", "単発", "单次日期")}
            </label>
          </fieldset>
          {draft.date ? (
            <label className="wide">
              {tr(locale, "Date", "日付", "日期")}
              <UiInput
                required
                type="date"
                value={draft.date}
                onChange={(e) => setDraft({ ...draft, date: e.target.value })}
              />
            </label>
          ) : (
            <fieldset className="weekdayPicker wide">
              <legend>
                {tr(locale, "Repeat on", "繰り返す曜日", "重复日期")}
              </legend>
              {weekdays.map((label, index) => {
                const day = index + 1;
                return (
                  <label key={label}>
                    <UiInput
                      type="checkbox"
                      checked={draft.days?.includes(day) || false}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          days: e.target.checked
                            ? [...(draft.days || []), day].sort()
                            : draft.days?.filter((item) => item !== day),
                        })
                      }
                    />
                    {label.slice(0, 3)}
                  </label>
                );
              })}
            </fieldset>
          )}
          <label>
            {tr(locale, "Starts", "開始", "开始")}
            <UiInput
              required
              type="time"
              value={draft.startTime}
              onChange={(e) =>
                setDraft({ ...draft, startTime: e.target.value })
              }
            />
          </label>
          <label>
            {tr(locale, "Ends", "終了", "结束")}
            <UiInput
              required
              type="time"
              value={draft.endTime}
              onChange={(e) => setDraft({ ...draft, endTime: e.target.value })}
            />
          </label>
          <label className="inlineChoice wide">
            <UiInput
              type="checkbox"
              checked={draft.blocking}
              onChange={(e) => {
                setDraft({ ...draft, blocking: e.target.checked });
                setConflictWarning(null);
              }}
            />{" "}
            {tr(
              locale,
              "Include this event in schedule conflict checks",
              "この予定を時間割の重複確認に含める",
              "将此日程纳入课程冲突检查",
            )}
          </label>
          {conflictWarning && (
            <p className="conflictWarning wide" role="alert">
              {tr(locale, "Overlaps", "重複先:", "冲突项：")}{" "}
              <strong>{conflictWarning}</strong>.{" "}
              {tr(
                locale,
                "Saving again will keep the overlap.",
                "もう一度保存すると重複を保持します。",
                "再次保存将保留此冲突。",
              )}
            </p>
          )}
          <div className="formActions wide">
            {editing !== "new" && (
              <UiButton
                className="button dangerText"
                type="button"
                onClick={() => remove(editing)}
              >
                {tr(locale, "Delete", "削除", "删除")}
              </UiButton>
            )}
            <UiButton
              className="button"
              type="button"
              onClick={() => setEditing(null)}
            >
              {tr(locale, "Cancel", "キャンセル", "取消")}
            </UiButton>
            <UiButton className="button primary" type="submit">
              {conflictWarning
                ? tr(
                    locale,
                    "Save despite conflict",
                    "重複したまま保存",
                    "仍然保存",
                  )
                : tr(locale, "Save event", "予定を保存", "保存日程")}
            </UiButton>
          </div>
        </form>
      )}
    </section>
  );
}
import { UiButton, UiInput } from "../../components/mantine";
