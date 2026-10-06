import { useRef, useState, type ChangeEvent, type FormEvent } from "react";
import { validateAcademicProfile } from "../../core/academic-calendar";
import type { CalendarException, Workspace } from "../../core/model";
import { newUserRecordId } from "../../storage/ids";
import "./events.css";

type Props = {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  notify: (message: string) => void;
  readOnly: boolean;
};
type Draft = Pick<
  CalendarException,
  "date" | "type" | "replacementDay" | "label"
>;
const blank: Draft = { date: "", type: "no-class", label: "" };
type Locale = Workspace["settings"]["locale"];
const tr = (locale: Locale, en: string, ja: string, zh: string) =>
  locale === "ja" ? ja : locale === "zh" ? zh : en;
const dayNames = (locale: Locale) =>
  locale === "ja"
    ? ["", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日"]
    : locale === "zh"
      ? ["", "星期一", "星期二", "星期三", "星期四", "星期五"]
      : ["", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];

function isCalendarDate(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return (
    parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day
  );
}

function validate(value: unknown, locale: Locale): Draft[] {
  const root = value as {
    schema?: unknown;
    version?: unknown;
    exceptions?: unknown;
  };
  if (!root || !Array.isArray(root.exceptions))
    throw new Error(
      tr(
        locale,
        "JSON must contain an exceptions array",
        "JSONにはexceptions配列が必要です",
        "JSON 必须包含 exceptions 数组",
      ),
    );
  if (root.schema !== undefined && root.schema !== "isct-academic-calendar")
    throw new Error(
      tr(
        locale,
        "Unsupported calendar schema",
        "対応していないカレンダー形式です",
        "不支持的校历格式",
      ),
    );
  if (root.version !== undefined && root.version !== 1)
    throw new Error(
      tr(
        locale,
        "Unsupported calendar version",
        "対応していないカレンダーバージョンです",
        "不支持的校历版本",
      ),
    );
  const dates = new Set<string>();
  return root.exceptions.map((raw, index) => {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new Error(
        tr(
          locale,
          `Exception ${index + 1} must be an object`,
          `例外 ${index + 1} はオブジェクトである必要があります`,
          `第 ${index + 1} 条例外必须是对象`,
        ),
      );
    const item = raw as Record<string, unknown>;
    const date = String(item.date || "");
    if (!isCalendarDate(date))
      throw new Error(
        tr(
          locale,
          `Exception ${index + 1} has an invalid date`,
          `例外 ${index + 1} の日付が無効です`,
          `第 ${index + 1} 条例外的日期无效`,
        ),
      );
    if (dates.has(date))
      throw new Error(
        tr(
          locale,
          `Date ${date} appears more than once`,
          `${date} が重複しています`,
          `日期 ${date} 重复`,
        ),
      );
    dates.add(date);
    if (!["no-class", "replacement", "break"].includes(String(item.type)))
      throw new Error(
        tr(
          locale,
          `Exception ${index + 1} has an invalid type`,
          `例外 ${index + 1} の種類が無効です`,
          `第 ${index + 1} 条例外的类型无效`,
        ),
      );
    const replacementDay =
      item.type === "replacement" ? Number(item.replacementDay) : undefined;
    if (
      item.type === "replacement" &&
      (!Number.isInteger(replacementDay) ||
        replacementDay! < 1 ||
        replacementDay! > 5)
    )
      throw new Error(
        tr(
          locale,
          `Exception ${index + 1} needs a replacement weekday`,
          `例外 ${index + 1} には振替曜日が必要です`,
          `第 ${index + 1} 条例外需要指定补课星期`,
        ),
      );
    if (typeof item.label !== "string" || !item.label.trim())
      throw new Error(
        tr(
          locale,
          `Exception ${index + 1} needs a label`,
          `例外 ${index + 1} には名称が必要です`,
          `第 ${index + 1} 条例外需要名称`,
        ),
      );
    return {
      date,
      type: item.type as Draft["type"],
      replacementDay,
      label: item.label.trim(),
    };
  });
}

export function CalendarManager({ data, update, notify, readOnly }: Props) {
  const locale = data.settings.locale;
  const weekdays = dayNames(locale);
  const languageTag = locale === "zh" ? "zh-CN" : locale;
  const formatDate = (date: string) =>
    new Intl.DateTimeFormat(languageTag, {
      month: "short",
      day: "numeric",
      timeZone: "Asia/Tokyo",
    }).format(new Date(`${date}T12:00:00+09:00`));
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const [draft, setDraft] = useState<Draft>(blank);
  const file = useRef<HTMLInputElement>(null);
  const exceptions = data.exceptions
    .filter((item) => !item.deletedAt)
    .sort((a, b) => a.date.localeCompare(b.date));
  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (readOnly) return;
    if (
      !isCalendarDate(draft.date) ||
      !draft.label.trim() ||
      (draft.type === "replacement" && !draft.replacementDay)
    ) {
      notify(
        tr(
          locale,
          "Complete the calendar exception",
          "カレンダー例外を入力してください",
          "请完整填写校历例外",
        ),
      );
      return;
    }
    const duplicate = exceptions.some(
      (item) => item.date === draft.date && item.id !== editing,
    );
    if (duplicate) {
      notify(
        tr(
          locale,
          "That date already has an exception. Edit it instead.",
          "その日付には既に例外があります。既存の項目を編集してください。",
          "该日期已有例外，请编辑现有项目。",
        ),
      );
      return;
    }
    const now = new Date().toISOString();
    const exceptionId = editing === "new" ? await newUserRecordId() : "";
    update((workspace) => ({
      ...workspace,
      exceptions:
        editing === "new"
          ? [
              ...workspace.exceptions,
              {
                ...draft,
                replacementDay:
                  draft.type === "replacement"
                    ? draft.replacementDay
                    : undefined,
                id: exceptionId,
                kind: "exception",
                rev: 1,
                updatedAt: now,
                updatedBy: workspace.deviceId,
              },
            ]
          : workspace.exceptions.map((item) =>
              item.id === editing
                ? {
                    ...item,
                    ...draft,
                    replacementDay:
                      draft.type === "replacement"
                        ? draft.replacementDay
                        : undefined,
                    rev: item.rev + 1,
                    updatedAt: now,
                    updatedBy: workspace.deviceId,
                  }
                : item,
            ),
    }));
    setEditing(null);
    notify(
      tr(
        locale,
        "Academic calendar updated",
        "学年暦を更新しました",
        "校历已更新",
      ),
    );
  };
  const remove = (id: string) => {
    if (readOnly) return;
    const now = new Date().toISOString();
    update((workspace) => ({
      ...workspace,
      exceptions: workspace.exceptions.map((item) =>
        item.id === id
          ? {
              ...item,
              deletedAt: now,
              rev: item.rev + 1,
              updatedAt: now,
              updatedBy: workspace.deviceId,
            }
          : item,
      ),
    }));
    setEditing(null);
    notify(
      tr(
        locale,
        "Calendar exception removed",
        "カレンダー例外を削除しました",
        "校历例外已删除",
      ),
    );
  };
  const beginEdit = (item: CalendarException) => {
    if (!readOnly) {
      setDraft({
        date: item.date,
        type: item.type,
        replacementDay: item.replacementDay,
        label: item.label,
      });
      setEditing(item.id);
    }
  };
  const importJson = async (event: ChangeEvent<HTMLInputElement>) => {
    const selected = event.target.files?.[0];
    event.target.value = "";
    if (!selected || readOnly) return;
    try {
      const parsed = JSON.parse(await selected.text()) as {
        profile?: unknown;
      };
      const incoming = validate(parsed, locale);
      let incomingProfile = data.academicProfile;
      if (parsed.profile !== undefined) {
        try {
          incomingProfile = validateAcademicProfile(parsed.profile);
        } catch {
          throw new Error(
            tr(
              locale,
              "Imported academic profile is invalid",
              "読み込んだ履修プロファイルが無効です",
              "导入的学业配置无效",
            ),
          );
        }
      }
      const now = new Date().toISOString();
      const incomingIds = await Promise.all(incoming.map(() => newUserRecordId()));
      update((workspace) => {
        const retained = workspace.exceptions.filter(
          (item) =>
            item.deletedAt || !incoming.some((next) => next.date === item.date),
        );
        return {
          ...workspace,
          academicProfile: incomingProfile,
          exceptions: [
            ...retained,
            ...incoming.map((item, index) => ({
              ...item,
              id: incomingIds[index],
              kind: "exception" as const,
              rev: 1,
              updatedAt: now,
              updatedBy: workspace.deviceId,
            })),
          ],
        };
      });
      notify(
        tr(
          locale,
          `${incoming.length} calendar exceptions imported`,
          `${incoming.length}件のカレンダー例外を読み込みました`,
          `已导入 ${incoming.length} 条校历例外`,
        ),
      );
    } catch (error) {
      notify(
        error instanceof Error
          ? error.message
          : tr(
              locale,
              "Calendar import failed",
              "カレンダーの読み込みに失敗しました",
              "校历导入失败",
            ),
      );
    }
  };
  const exportJson = () => {
    const years = [
      ...new Set(exceptions.map((item) => Number(item.date.slice(0, 4)))),
    ].sort();
    const payload = {
      schema: "isct-academic-calendar",
      version: 1,
      years,
      exportedAt: new Date().toISOString(),
      profile: data.academicProfile,
      exceptions: exceptions.map(({ date, type, replacementDay, label }) => ({
        date,
        type,
        ...(replacementDay ? { replacementDay } : {}),
        label,
      })),
    };
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(payload, null, 2)], {
        type: "application/json",
      }),
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `academic-calendar-${years.join("-") || "empty"}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
    notify(
      tr(
        locale,
        "Academic calendar exported",
        "学年暦を書き出しました",
        "校历已导出",
      ),
    );
  };
  return (
    <section
      className="surface featureManager"
      aria-labelledby="calendar-title"
    >
      <div className="surfaceHead featureHead">
        <div>
          <h2 id="calendar-title">
            {tr(locale, "Academic calendar", "学年暦", "校历")}
          </h2>
        </div>
        <div className="headActions">
          {!readOnly && (
            <>
              <UiInput
                ref={file}
                hidden
                type="file"
                accept="application/json,.json"
                onChange={importJson}
              />
              <UiButton
                className="button primary"
                type="button"
                onClick={() => {
                  setDraft(blank);
                  setEditing("new");
                }}
              >
                ＋ {tr(locale, "Add", "追加", "添加")}
              </UiButton>
            </>
          )}
          <details className="overflowMenu">
            <summary
              aria-label={tr(locale, "More actions", "その他", "更多操作")}
            >
              ⋯
            </summary>
            <div>
              <UiButton type="button" onClick={exportJson}>
                {tr(locale, "Export", "書き出し", "导出")}
              </UiButton>
              {!readOnly && (
                <>
                  <UiButton type="button" onClick={() => file.current?.click()}>
                    {tr(locale, "Import", "読み込み", "导入")}
                  </UiButton>
                </>
              )}
            </div>
          </details>
        </div>
      </div>
      {exceptions.length ? (
        <div className="eventList">
          {exceptions.map((item) => (
            <article className="eventRow calendarRow" key={item.id}>
              <time>{formatDate(item.date)}</time>
              <div>
                <strong>{item.label}</strong>
                <small>
                  {item.type === "replacement"
                    ? tr(
                        locale,
                        `Replacement ${weekdays[item.replacementDay || 0]} timetable`,
                        `${weekdays[item.replacementDay || 0]}の時間割で実施`,
                        `按${weekdays[item.replacementDay || 0]}课表上课`,
                      )
                    : item.type === "no-class"
                      ? tr(locale, "No classes", "休講", "停课")
                      : tr(locale, "Academic break", "学事休暇", "教学假期")}
                </small>
              </div>
              {!readOnly && (
                <div className="rowActions">
                  <UiButton type="button" onClick={() => beginEdit(item)}>
                    {tr(locale, "Edit", "編集", "编辑")}
                  </UiButton>
                  <UiButton
                    className="dangerText"
                    type="button"
                    onClick={() => remove(item.id)}
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
            "No exceptions",
            "例外はありません",
            "暂无例外",
          )}
        </p>
      )}
      {editing && !readOnly && (
        <form className="featureForm calendarForm" onSubmit={save}>
          <div className="formTitle wide">
            <strong>
              {editing === "new"
                ? tr(
                    locale,
                    "New exception",
                    "新しい例外",
                    "新建例外",
                  )
                : tr(
                    locale,
                    "Edit exception",
                    "例外を編集",
                    "编辑例外",
                  )}
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
          <label>
            {tr(locale, "Date", "日付", "日期")}
            <UiInput
              required
              type="date"
              value={draft.date}
              onChange={(e) => setDraft({ ...draft, date: e.target.value })}
            />
          </label>
          <label>
            {tr(locale, "Type", "種類", "类型")}
            <UiSelect
              value={draft.type}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  type: e.target.value as Draft["type"],
                  replacementDay:
                    e.target.value === "replacement"
                      ? draft.replacementDay || 1
                      : undefined,
                })
              }
            >
              <option value="no-class">
                {tr(locale, "No classes", "休講", "停课")}
              </option>
              <option value="break">
                {tr(locale, "Academic break", "学事休暇", "教学假期")}
              </option>
              <option value="replacement">
                {tr(locale, "Replacement day", "振替授業日", "补课日")}
              </option>
            </UiSelect>
          </label>
          {draft.type === "replacement" && (
            <label>
              {tr(locale, "Use timetable", "使用する時間割", "使用课表")}
              <UiSelect
                value={draft.replacementDay}
                onChange={(e) =>
                  setDraft({ ...draft, replacementDay: Number(e.target.value) })
                }
              >
                {weekdaysForSelect(weekdays)}
              </UiSelect>
            </label>
          )}
          <label className={draft.type === "replacement" ? "" : "wide"}>
            {tr(locale, "Label", "名称", "名称")}
            <UiInput
              required
              value={draft.label}
              onChange={(e) => setDraft({ ...draft, label: e.target.value })}
              placeholder={tr(
                locale,
                "University Foundation Day",
                "創立記念日",
                "建校纪念日",
              )}
            />
          </label>
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
              {tr(locale, "Save", "保存", "保存")}
            </UiButton>
          </div>
        </form>
      )}
    </section>
  );
}

function weekdaysForSelect(names: string[]) {
  return names.slice(1).map((name, index) => (
    <option value={index + 1} key={name}>
      {name}
    </option>
  ));
}
import {
  UiButton,
  UiInput,
  UiSelect,
  UiTextarea,
} from "../../components/mantine";
