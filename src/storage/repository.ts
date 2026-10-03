import { invoke } from "@tauri-apps/api/core";
import { clockForPeriods } from "../core/user-schedule";
import { seedWorkspace, type Workspace } from "../core/model";

const KEY = "stws-workspace-v3";

export type ExportDocument = {
  schemaVersion: 3;
  exportedAt: string;
  deviceId: string;
  tables: Record<string, unknown>;
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
  // Meetings saved without clock times (e.g. a 5-8 period block) get them from their periods.
  for (const course of data.courses ?? [])
    for (const meeting of course.meetings ?? []) {
      if (!CLOCK.test(meeting.startTime) || !CLOCK.test(meeting.endTime)) {
        const periods = meeting.periods?.filter(Number.isFinite) ?? [];
        if (periods.length) {
          const clock = clockForPeriods([Math.min(...periods), Math.max(...periods)]);
          meeting.startTime = clock.start;
          meeting.endTime = clock.end;
        }
      }
    }
  data.terms ??= [];
  data.planItems ??= [];
  data.scheduledTasks ??= [];
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
  const settings = {
    selectedTerm: workspace.selectedTerm,
    terms: workspace.terms ?? [],
    settings: workspace.settings,
    academicProfile: workspace.academicProfile,
    importStatus: workspace.importStatus,
    syncStatus: workspace.syncStatus,
    logs: workspace.logs ?? [],
  };
  return {
    schemaVersion: 3,
    exportedAt,
    deviceId: workspace.deviceId,
    tables: {
      timetable_courses: workspace.courses,
      memberships: workspace.memberships,
      assignments: workspace.assignments.map(
        ({ hidden: _hidden, completedAt: _completedAt, ...assignment }) =>
          assignment,
      ),
      lms_items: workspace.materials,
      personal_events: workspace.events,
      plan_items: workspace.planItems,
      calendar_exceptions: workspace.exceptions,
      offerings: workspace.offerings,
      intensive_schedules: workspace.intensiveSchedules,
      plans: workspace.plans,
      academic_calendar: workspace.academicCalendar,
      gpa_summary: workspace.gpaSummary,
      departments: workspace.departments,
      recommended_courses: workspace.recommendedCourses,
      subscriptions: workspace.subscriptions,
      scheduled_tasks: workspace.scheduledTasks,
      course_overrides: [],
      assignment_marks: workspace.assignments
        .filter((assignment) => assignment.hidden || assignment.completedAt)
        .map((assignment) => ({
          id: `assignment-mark:${assignment.id}`,
          assignmentId: assignment.id,
          hidden: Boolean(assignment.hidden),
          completedAt: assignment.completedAt,
          rev: assignment.rev,
          updatedAt: assignment.updatedAt,
          updatedBy: assignment.updatedBy,
          source: { provider: "manual" },
        })),
      registrations: [],
      registration_states: [],
      grades: [],
      requirement_progress: [],
      lms_courses: [],
      gpa_by_term: [],
      settings: Object.entries(settings).map(([key, value]) => ({
        key,
        value,
        rev: 1,
        updatedAt: exportedAt,
        updatedBy: workspace.deviceId,
      })),
    },
  };
}

const ARRAY_TABLES = [
  "timetable_courses",
  "course_overrides",
  "plan_items",
  "memberships",
  "personal_events",
  "calendar_exceptions",
  "assignments",
  "assignment_marks",
  "lms_courses",
  "lms_items",
  "registrations",
  "registration_states",
  "grades",
  "requirement_progress",
  "intensive_schedules",
  "plans",
  "gpa_summary",
  "gpa_by_term",
  "subscriptions",
  "academic_calendar",
  "departments",
  "recommended_courses",
  "scheduled_tasks",
  "offerings",
  "settings",
] as const;

function validateExportDocument(value: unknown): ExportDocument {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new UnsupportedWorkspaceSchemaError("Export must be an object.");
  const document = value as Partial<ExportDocument>;
  if (
    document.schemaVersion !== 3 ||
    typeof document.deviceId !== "string" ||
    !document.deviceId ||
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
  const marks = new Map(
    (document.tables.assignment_marks as Array<{
      assignmentId: string;
      hidden?: boolean;
      completedAt?: string;
      deletedAt?: string;
    }>).filter((mark) => !mark.deletedAt).map((mark) => [mark.assignmentId, mark]),
  );
  const workspace = {
    schemaVersion: 3,
    deviceId: document.deviceId,
    selectedTerm: value.selectedTerm,
    terms: value.terms,
    courses: document.tables.timetable_courses,
    memberships: document.tables.memberships,
    assignments: (document.tables.assignments as Workspace["assignments"]).map(
      (assignment) => {
        const mark = marks.get(assignment.id);
        return mark
          ? {
              ...assignment,
              hidden: mark.hidden || undefined,
              completedAt: mark.completedAt,
            }
          : assignment;
      },
    ),
    materials: document.tables.lms_items,
    events: document.tables.personal_events,
    planItems: document.tables.plan_items,
    exceptions: document.tables.calendar_exceptions,
    offerings: document.tables.offerings ?? [],
    intensiveSchedules: document.tables.intensive_schedules ?? [],
    plans: document.tables.plans ?? [],
    academicCalendar: document.tables.academic_calendar ?? [],
    gpaSummary: document.tables.gpa_summary ?? [],
    departments: document.tables.departments ?? [],
    recommendedCourses: document.tables.recommended_courses ?? [],
    subscriptions: document.tables.subscriptions ?? [],
    scheduledTasks: document.tables.scheduled_tasks ?? [],
    settings: value.settings,
    academicProfile: value.academicProfile,
    importStatus: value.importStatus,
    syncStatus: value.syncStatus,
    logs: value.logs,
  } as Workspace;
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
  async load() {
    return normalize(await invoke<Workspace>("load_workspace"));
  }

  async save(data: Workspace) {
    await invoke("save_workspace", { workspace: data });
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
