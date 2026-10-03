// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace } from "../core/model";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => {
    throw new Error("Native commands are outside this browser-preview contract.");
  }),
}));

import {
  LocalRepository,
  UnsupportedWorkspaceSchemaError,
  type ExportDocument,
} from "../storage/repository";

const STORAGE_KEY = "stws-workspace-v3";

function validDocument(): ExportDocument {
  const workspace = seedWorkspace();
  return {
    schemaVersion: 3,
    exportedAt: "2026-09-29T00:00:00.000Z",
    deviceId: "synthetic-device",
    tables: {
      timetable_courses: [],
      memberships: [],
      assignments: [],
      lms_items: [],
      personal_events: [],
      plan_items: [],
      calendar_exceptions: [],
      offerings: [],
      intensive_schedules: [],
      plans: [],
      academic_calendar: [],
      gpa_summary: [],
      departments: [],
      recommended_courses: [],
      subscriptions: [],
      scheduled_tasks: [],
      settings: {
        selectedTerm: workspace.selectedTerm,
        terms: workspace.terms,
        settings: workspace.settings,
        academicProfile: workspace.academicProfile,
        importStatus: workspace.importStatus,
        syncStatus: workspace.syncStatus,
        logs: [],
      },
    },
  };
}

describe("Task 1 browser import validation", () => {
  beforeEach(() => localStorage.clear());

  it("validates every required collection before replacing local data", async () => {
    const repository = new LocalRepository();
    const current = seedWorkspace();
    current.settings.appearance = "dark";
    await repository.save(current);
    const before = localStorage.getItem(STORAGE_KEY);

    const incomplete = validDocument();
    delete incomplete.tables.timetable_courses;

    await expect(repository.import(JSON.stringify(incomplete))).rejects.toBeInstanceOf(
      UnsupportedWorkspaceSchemaError,
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBe(before);
  });

  it("rejects a malformed table instead of persisting a corrupt workspace", async () => {
    const repository = new LocalRepository();
    const current = seedWorkspace();
    await repository.save(current);
    const before = localStorage.getItem(STORAGE_KEY);
    const malformed = validDocument();
    malformed.tables.assignments = { id: "not-an-array" };

    await expect(repository.import(JSON.stringify(malformed))).rejects.toBeInstanceOf(
      UnsupportedWorkspaceSchemaError,
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBe(before);
  });

  it("requires export metadata before accepting the shared document", async () => {
    const repository = new LocalRepository();
    const malformed = validDocument() as ExportDocument & {
      exportedAt?: string;
    };
    delete malformed.exportedAt;

    await expect(repository.import(JSON.stringify(malformed))).rejects.toBeInstanceOf(
      UnsupportedWorkspaceSchemaError,
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });
});
