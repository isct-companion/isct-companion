import { invoke } from "@tauri-apps/api/core";
import { clockForPeriods } from "../core/user-schedule";
import {
  LMS_BACKGROUND_REFRESH_TASK_ID,
  REFERENCE_REFRESH_TASK_ID,
  lmsBackgroundRefreshTask,
  referenceRefreshTask,
  seedWorkspace,
  type Workspace,
} from "../core/model";

const KEY = "isct-companion-workspace-v3";

export type ExportDocument = {
  schemaVersion: 6;
  exportedAt: string;
  kind: "IsctCompanion.local-backup";
  /** Format metadata only; never the device preference stored in SQLite. */
  deviceId: string;
  tables: Record<string, unknown>;
  localNotes: unknown[];
};

export interface WorkspaceRepository {
  load(): Promise<Workspace>;
  save(data: Workspace): Promise<void>;
  export(): Promise<string>;
  import?(document: string): Promise<Workspace>;
  reset(): Promise<Workspace>;
}

export class UnsupportedWorkspaceSchemaError extends Error {
  override name = "UnsupportedWorkspaceSchemaError";
}

const CLOCK = /^\d{1,2}:\d{2}$/;

function normalize(data: Workspace): Workspace {
  data.currentTerm ??= data.selectedTerm;
  // Meetings saved without clock times (e.g. a 5-8 period block) get them from their periods.
  for (const course of data.courses ?? [])
    for (const meeting of course.meetings ?? []) {
      if (!CLOCK.test(meeting.startTime) || !CLOCK.test(meeting.endTime)) {
        const periods = meeting.periods?.filter(Number.isFinite) ?? [];
        if (periods.length) {
          const clock = clockForPeriods([
            Math.min(...periods),
            Math.max(...periods),
          ]);
          meeting.startTime = clock.start;
          meeting.endTime = clock.end;
        }
      }
    }
  data.terms ??= [];
  data.planItems ??= [];
  data.scheduledTasks ??= [];
  if (
    !data.scheduledTasks.some((task) => task.id === REFERENCE_REFRESH_TASK_ID)
  )
    data.scheduledTasks.push(referenceRefreshTask(data.deviceId));
  if (
    !data.scheduledTasks.some(
      (task) => task.id === LMS_BACKGROUND_REFRESH_TASK_ID,
    )
  )
    data.scheduledTasks.push(lmsBackgroundRefreshTask(data.deviceId));
  data.importantDates ??= [];
  data.exams ??= [];
  data.scheduleOverrides ??= [];
  data.settings.subscriptionRefresh ??= {
    frequency: "daily",
    allowCellular: false,
  };
  data.academicProfile ??= seedWorkspace().academicProfile;
  if (
    !["keep-current", "use-latest", "resolve-manually"].includes(
      data.settings.cloud.conflict,
    )
  )
    data.settings.cloud.conflict = "resolve-manually";
  data.settings.customization ??= {
    background: "",
    sites: {
      portal: { css: "", js: "", enabled: false },
      lms: { css: "", js: "", enabled: false },
      syllabus: { css: "", js: "", enabled: false },
    },
    hooks: [],
  };
  data.settings.cloud.redirectUri ??= "http://127.0.0.1:42813/oauth/callback";
  return data;
}

function localExport(workspace: Workspace): ExportDocument {
  const exportedAt = new Date().toISOString();
  const versionedSetting = (key: string, value: unknown) => ({
    key,
    value,
    rev: 1,
    updatedAt: exportedAt,
    updatedBy: workspace.deviceId,
  });
  return {
    schemaVersion: 6,
    exportedAt,
    kind: "IsctCompanion.local-backup",
    deviceId: "local-backup",
    localNotes: [],
    tables: {
      manual_courses: workspace.courses.filter((course) => !course.lastSeenAt),
      manual_memberships: workspace.memberships.filter(
        (membership) => !membership.lastSeenAt,
      ),
      personal_events: workspace.events,
      plan_items: workspace.planItems,
      calendar_exceptions: workspace.exceptions,
      intensive_schedules: workspace.intensiveSchedules,
      plans: workspace.plans,
      schedule_overrides: workspace.scheduleOverrides,
      subscriptions: workspace.subscriptions,
      course_overrides: workspace.courses
        .filter((course) => course.overrides)
        .map((course) => ({
          id: `course-override:${course.id}`,
          sourceKey: course.sourceKey ?? course.id,
          ...course.overrides,
          rev: course.rev,
          updatedAt: course.updatedAt,
          updatedBy: course.updatedBy,
        })),
      assignment_marks: workspace.assignments
        .filter((assignment) => assignment.hidden || assignment.completedAt)
        .map((assignment) => ({
          id: `assignment-mark:${assignment.id}`,
          sourceKey: assignment.sourceKey ?? assignment.id,
          hidden: Boolean(assignment.hidden),
          completedAt: assignment.completedAt,
          rev: assignment.rev,
          updatedAt: assignment.updatedAt,
          updatedBy: assignment.updatedBy,
        })),
      settings: [
        versionedSetting("userPreferences", {
          departmentId: workspace.settings.departmentId,
          notifications: {
            classes: workspace.settings.notifications.classes,
            assignments: workspace.settings.notifications.assignments,
            classLeadMinutes: workspace.settings.notifications.classLeadMinutes,
          },
          customization: workspace.settings.customization,
        }),
        versionedSetting("academicProfile", workspace.academicProfile),
      ],
    },
  };
}

const ARRAY_TABLES = [
  "manual_courses",
  "manual_memberships",
  "course_overrides",
  "plan_items",
  "personal_events",
  "calendar_exceptions",
  "assignment_marks",
  "intensive_schedules",
  "plans",
  "schedule_overrides",
  "subscriptions",
  "settings",
] as const;

export function validateExportDocument(value: unknown): ExportDocument {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new UnsupportedWorkspaceSchemaError("Export must be an object.");
  const document = value as Partial<ExportDocument>;
  if (
    document.schemaVersion !== 6 ||
    document.kind !== "IsctCompanion.local-backup" ||
    typeof document.exportedAt !== "string" ||
    Number.isNaN(Date.parse(document.exportedAt)) ||
    !document.tables ||
    typeof document.tables !== "object" ||
    Array.isArray(document.tables)
  )
    throw new UnsupportedWorkspaceSchemaError(
      "Export schema is unsupported or incomplete.",
    );
  for (const table of ARRAY_TABLES) {
    if (!Array.isArray(document.tables[table]))
      throw new UnsupportedWorkspaceSchemaError(
        `Export table ${table} must be an array.`,
      );
  }
  if (!Array.isArray(document.localNotes))
    throw new UnsupportedWorkspaceSchemaError(
      "Export localNotes must be an array.",
    );
  for (const row of document.tables.settings as unknown[]) {
    if (
      !row ||
      typeof row !== "object" ||
      Array.isArray(row) ||
      typeof (row as { key?: unknown }).key !== "string" ||
      !(row as { value?: unknown }).hasOwnProperty("value")
    )
      throw new UnsupportedWorkspaceSchemaError("Export settings are invalid.");
  }
  return document as ExportDocument;
}

function workspaceFromLocalExport(document: ExportDocument): Workspace {
  const value = Object.fromEntries(
    (document.tables.settings as Array<{ key: string; value: unknown }>).map(
      (row) => [row.key, row.value],
    ),
  );
  const seed = seedWorkspace();
  const preferences = (value.userPreferences ?? {}) as Partial<
    Workspace["settings"]
  >;
  const workspace = {
    ...seed,
    courses: document.tables.manual_courses,
    memberships: document.tables.manual_memberships,
    assignments: [],
    materials: [],
    events: document.tables.personal_events,
    planItems: document.tables.plan_items,
    exceptions: document.tables.calendar_exceptions,
    offerings: [],
    intensiveSchedules: document.tables.intensive_schedules,
    plans: document.tables.plans,
    academicCalendar: [],
    gpaSummary: [],
    departments: [],
    recommendedCourses: [],
    importantDates: [],
    exams: [],
    scheduleOverrides: document.tables.schedule_overrides ?? [],
    subscriptions: document.tables.subscriptions ?? [],
    scheduledTasks: [],
    settings: {
      ...seed.settings,
      ...preferences,
      notifications: {
        ...seed.settings.notifications,
        ...(preferences.notifications ?? {}),
      },
    },
    academicProfile: value.academicProfile ?? seed.academicProfile,
  } as unknown as Workspace;
  return normalize(workspace);
}

export class LocalRepository implements WorkspaceRepository {
  async load() {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      try {
        const value = JSON.parse(raw);
        if (value.schemaVersion === 3) return normalize(value as Workspace);
        throw new UnsupportedWorkspaceSchemaError(
          `Workspace schema ${value?.schemaVersion ?? "unknown"} is unsupported. Stored data was preserved.`,
        );
      } catch (error) {
        if (error instanceof UnsupportedWorkspaceSchemaError) throw error;
        throw new UnsupportedWorkspaceSchemaError(
          "Workspace data is corrupt. Stored bytes were preserved.",
        );
      }
    }
    const data = seedWorkspace();
    await this.save(data);
    return data;
  }

  async save(data: Workspace) {
    localStorage.setItem(KEY, JSON.stringify(data));
  }

  async export() {
    return JSON.stringify(localExport(await this.load()), null, 2);
  }

  async import(document: string) {
    let parsed: ExportDocument;
    try {
      parsed = validateExportDocument(JSON.parse(document));
    } catch {
      throw new UnsupportedWorkspaceSchemaError("Export is not valid JSON.");
    }
    const workspace = workspaceFromLocalExport(parsed);
    await this.save(workspace);
    return workspace;
  }

  async reset() {
    const data = seedWorkspace();
    await this.save(data);
    return data;
  }
}

/** Desktop source of truth. Rust owns the SQLite database and all merge rules. */
export class SqliteRepository implements WorkspaceRepository {
  private baseline?: Workspace;

  async load() {
    const workspace = normalize(await invoke<Workspace>("load_workspace"));
    this.baseline = structuredClone(workspace);
    return workspace;
  }

  async save(data: Workspace) {
    if (!this.baseline) await this.load();
    const previous = this.baseline!;
    const same = (left: unknown, right: unknown) =>
      JSON.stringify(left) === JSON.stringify(right);
    // SQLite owns rev/updatedAt/updatedBy. Local edits bump them too, so a
    // record whose content matches the stored copy must not be re-sent.
    const content = (record: object | undefined) => {
      if (!record) return undefined;
      const { rev: _rev, updatedAt: _at, updatedBy: _by, ...rest } =
        record as Record<string, unknown>;
      return rest;
    };
    const collections = [
      ["courses", "manual_courses"],
      ["memberships", "manual_memberships"],
      ["events", "personal_events"],
      ["planItems", "plan_items"],
      ["exceptions", "calendar_exceptions"],
      ["intensiveSchedules", "intensive_schedules"],
      ["plans", "plans"],
      ["scheduleOverrides", "schedule_overrides"],
      ["subscriptions", "subscriptions"],
      ["scheduledTasks", "scheduled_tasks"],
    ] as const;
    const schoolCopy = (field: string, record: object) =>
      (field === "courses" || field === "memberships") &&
      "lastSeenAt" in record &&
      Boolean(record.lastSeenAt);
    const persisted = new Map<string, Map<string, object>>();
    for (const [field, table] of collections) {
      // `previous` is the live baseline: each write is recorded as it lands,
      // so a failure later in this save cannot strand rows on a stale rev.
      const stored: object[] = previous[field];
      const rows = new Map(
        stored.map((record) => [(record as { id: string }).id, record]),
      );
      persisted.set(field, rows);
      const remember = (record: { id: string }) => {
        const copy = structuredClone(record);
        const old = rows.get(record.id);
        if (old) stored[stored.indexOf(old)] = copy;
        else stored.push(copy);
        rows.set(record.id, copy);
      };
      for (const record of data[field]) {
        if (schoolCopy(field, record)) continue;
        const old = rows.get(record.id) as typeof record | undefined;
        if (same(content(old), content(record))) continue;
        if (record.deletedAt) {
          let rev = old?.rev ?? 0;
          if (!old?.deletedAt) {
            const deleted = await invoke<boolean>("delete_user_record", {
              table,
              id: record.id,
              expectedRev: rev,
            });
            if (deleted) rev += 1;
          }
          remember({ ...record, rev } as typeof record);
        } else {
          const saved = await invoke<typeof record>("put_user_record", {
            table,
            record,
            expectedRev: old?.rev ?? 0,
          });
          Object.assign(record, saved);
          remember(record);
        }
      }
    }

    const oldAssignments = new Map(
      previous.assignments.map((item) => [item.id, item]),
    );
    for (const assignment of data.assignments) {
      const old = oldAssignments.get(assignment.id);
      if (
        old?.hidden === assignment.hidden &&
        old?.completedAt === assignment.completedAt
      )
        continue;
      await invoke("put_user_record", {
        table: "assignment_marks",
        record: {
          sourceKey: assignment.sourceKey ?? assignment.id,
          assignmentId: assignment.id,
          hidden: Boolean(assignment.hidden),
          completedAt: assignment.completedAt,
          source: { provider: "manual" },
        },
      });
    }

    const oldCourses = new Map(previous.courses.map((item) => [item.id, item]));
    for (const course of data.courses) {
      if (same(oldCourses.get(course.id)?.overrides, course.overrides))
        continue;
      await invoke("put_user_record", {
        table: "course_overrides",
        record: {
          sourceKey: course.sourceKey ?? course.id,
          courseId: course.id,
          ...course.overrides,
          source: { provider: "manual" },
        },
      });
    }

    const userPreferences = {
      departmentId: data.settings.departmentId,
      notifications: {
        classes: data.settings.notifications.classes,
        assignments: data.settings.notifications.assignments,
        classLeadMinutes: data.settings.notifications.classLeadMinutes,
      },
      customization: data.settings.customization,
    };
    const oldUserPreferences = {
      departmentId: previous.settings.departmentId,
      notifications: {
        classes: previous.settings.notifications.classes,
        assignments: previous.settings.notifications.assignments,
        classLeadMinutes: previous.settings.notifications.classLeadMinutes,
      },
      customization: previous.settings.customization,
    };
    const devicePreferences = {
      lmsBilingualFilter: data.settings.lmsBilingualFilter,
      autoDownloadLimitMb: data.settings.autoDownloadLimitMb,
      autoCleanDays: data.settings.autoCleanDays,
      agentAccess: data.settings.agentAccess,
      appearance: data.settings.appearance,
      density: data.settings.density,
      textSize: data.settings.textSize,
      restoreSchoolPage: data.settings.restoreSchoolPage,
      activeSchoolSite: data.settings.activeSchoolSite,
      schoolServiceWidth: data.settings.schoolServiceWidth,
      schoolWebZoom: data.settings.schoolWebZoom,
      showGrades: data.settings.showGrades,
      siteConsents: data.settings.siteConsents,
      locale: data.settings.locale,
      contentLanguage: data.settings.contentLanguage,
      subscriptionRefresh: data.settings.subscriptionRefresh,
      notifications: {
        localPermission: data.settings.notifications.localPermission,
      },
    };
    const oldDevicePreferences = {
      lmsBilingualFilter: previous.settings.lmsBilingualFilter,
      autoDownloadLimitMb: previous.settings.autoDownloadLimitMb,
      autoCleanDays: previous.settings.autoCleanDays,
      agentAccess: previous.settings.agentAccess,
      appearance: previous.settings.appearance,
      density: previous.settings.density,
      textSize: previous.settings.textSize,
      restoreSchoolPage: previous.settings.restoreSchoolPage,
      activeSchoolSite: previous.settings.activeSchoolSite,
      schoolServiceWidth: previous.settings.schoolServiceWidth,
      schoolWebZoom: previous.settings.schoolWebZoom,
      showGrades: previous.settings.showGrades,
      siteConsents: previous.settings.siteConsents,
      locale: previous.settings.locale,
      contentLanguage: previous.settings.contentLanguage,
      subscriptionRefresh: previous.settings.subscriptionRefresh,
      notifications: {
        localPermission: previous.settings.notifications.localPermission,
      },
    };
    const settings = [
      ["selectedTerm", previous.selectedTerm, data.selectedTerm],
      ["terms", previous.terms, data.terms],
      ["academicProfile", previous.academicProfile, data.academicProfile],
      ["userPreferences", oldUserPreferences, userPreferences],
      ["devicePreferences", oldDevicePreferences, devicePreferences],
      ["cloud", previous.settings.cloud, data.settings.cloud],
      ["importStatus", previous.importStatus, data.importStatus],
      ["syncStatus", previous.syncStatus, data.syncStatus],
      ["logs", previous.logs, data.logs],
    ] as const;
    for (const [key, oldValue, value] of settings) {
      if (!same(oldValue, value)) {
        const currentTerm = await invoke<unknown>("put_workspace_setting", {
          key,
          value: value ?? null,
        });
        if (typeof currentTerm === "string")
          data.currentTerm = currentTerm as Workspace["currentTerm"];
      }
    }
    // Unsent records keep the stored metadata, not the locally bumped copy.
    const baseline = structuredClone(data);
    for (const [field] of collections)
      (baseline[field] as object[]) = baseline[field].map((record) =>
        schoolCopy(field, record)
          ? record
          : structuredClone(persisted.get(field)!.get(record.id) ?? record),
      );
    this.baseline = baseline;
  }

  async export() {
    return invoke<string>("export_data");
  }

  async import(document: string) {
    return normalize(await invoke<Workspace>("import_data", { document }));
  }

  async reset() {
    return normalize(await invoke<Workspace>("reset_workspace"));
  }
}

// LocalStorage exists only to keep the ordinary browser development preview usable.
export const repository: WorkspaceRepository =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
    ? new SqliteRepository()
    : new LocalRepository();
