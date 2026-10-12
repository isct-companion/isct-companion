import type { SchoolImportResult } from "../platform/capabilities";
import type {
  Assignment,
  Course,
  Material,
  Membership,
  Workspace,
} from "../core/model";

export function publishSchoolImport(
  workspace: Workspace,
  result: SchoolImportResult,
): Workspace {
  const next = structuredClone(workspace);
  next.logs = [
    {
      at: new Date().toISOString(),
      source: "school" as const,
      phase: "import",
      state: result.state,
      message: result.message.replace(/https?:\/\/\S+/g, "[redacted URL]"),
      retryable: result.state === "cached" || result.state === "error",
    },
    ...(next.logs ?? []),
  ].slice(0, 100);
  next.importStatus = {
    school:
      result.state === "fresh"
        ? "connected"
        : result.state === "reconnect-required"
          ? "reconnect-required"
          : result.state === "error"
            ? "error"
            : next.importStatus.school,
    lastImportedAt: result.fetchedAt ?? next.importStatus.lastImportedAt,
    message: result.message,
    lastResult: result.state,
  };
  if (result.state !== "fresh" || !result.fetchedAt) return next;
  const stamp = result.fetchedAt;
  const priorCourses = new Map(
    next.courses
      .filter((c) => c.source.provider === "school")
      .map((c) => [c.source.externalId, c]),
  );
  const priorMemberships = new Map(
    next.memberships
      .filter((m) => m.evidence.provider === "school")
      .map((m) => [m.evidence.externalId, m]),
  );
  const priorAssignments = new Map(
    next.assignments
      .filter((a) => a.source.provider === "school")
      .map((a) => [a.source.externalId, a]),
  );
  const priorMaterials = new Map(
    next.materials
      .filter((m) => m.source.provider === "school")
      .map((m) => [m.source.externalId, m]),
  );
  const courses: Course[] = [],
    memberships: Membership[] = [],
    assignments: Assignment[] = [],
    materials: Material[] = [];
  for (const remote of result.courses) {
    const old = priorCourses.get(remote.externalId),
      courseId = old?.id ?? `school-${remote.externalId}`;
    courses.push({
      id: courseId,
      kind: "course",
      rev: (old?.rev ?? 0) + 1,
      updatedAt: stamp,
      updatedBy: next.deviceId,
      termId: old?.termId ?? next.selectedTerm,
      code: remote.code,
      title: remote.title,
      instructor: old?.instructor ?? "Not provided by this source",
      credits: old?.credits ?? 0,
      creditSplit: old?.creditSplit ?? { lecture: old?.credits ?? 0, exercise: 0, lab: 0 },
      instructionLanguage: old?.instructionLanguage ?? "other",
      color: old?.color ?? "#526e91",
      meetings: old?.meetings ?? [],
      overrides: old?.overrides,
      source: {
        provider: "school",
        externalId: remote.externalId,
        fetchedAt: stamp,
      },
    });
    const oldMembership = priorMemberships.get(remote.externalId);
    memberships.push({
      id: oldMembership?.id ?? `school-membership-${remote.externalId}`,
      kind: "membership",
      rev: (oldMembership?.rev ?? 0) + 1,
      updatedAt: stamp,
      updatedBy: next.deviceId,
      termId: old?.termId ?? next.selectedTerm,
      courseId,
      status: "lms-member",
      evidence: {
        provider: "school",
        externalId: remote.externalId,
        fetchedAt: stamp,
      },
    });
    for (const item of remote.materials ?? []) {
      const previous = priorMaterials.get(item.externalId);
      materials.push({
        id: previous?.id ?? `school-material-${item.externalId}`,
        kind: "material",
        rev: (previous?.rev ?? 0) + 1,
        updatedAt: stamp,
        updatedBy: next.deviceId,
        courseId,
        session: previous?.session ?? 0,
        title: item.title,
        type: previous?.type ?? "link",
        url: item.url,
        source: {
          provider: "school",
          externalId: item.externalId,
          fetchedAt: stamp,
        },
      });
    }
    for (const item of remote.assignments ?? []) {
      if (!item.dueAt) continue;
      const previous = priorAssignments.get(item.externalId);
      assignments.push({
        id: previous?.id ?? `school-assignment-${item.externalId}`,
        kind: "assignment",
        rev: (previous?.rev ?? 0) + 1,
        updatedAt: stamp,
        updatedBy: next.deviceId,
        courseId,
        title: item.title,
        details:
          previous?.details ??
          "Open the original LMS page for authoritative details.",
        dueAt: item.dueAt,
        sourceUrl: item.url,
        hidden: previous?.hidden,
        pinned: previous?.pinned,
        source: {
          provider: "school",
          externalId: item.externalId,
          fetchedAt: stamp,
        },
      });
    }
  }
  const seenCourses = new Set(courses.map((c) => c.source.externalId));
  for (const old of priorCourses.values())
    if (!seenCourses.has(old.source.externalId)) courses.push(old);
  const seenMemberships = new Set(
    memberships.map((m) => m.evidence.externalId),
  );
  for (const old of priorMemberships.values())
    if (!seenMemberships.has(old.evidence.externalId)) memberships.push(old);
  const seenAssignments = new Set(assignments.map((a) => a.source.externalId));
  for (const old of priorAssignments.values())
    if (!seenAssignments.has(old.source.externalId)) assignments.push(old);
  const seenMaterials = new Set(materials.map((m) => m.source.externalId));
  for (const old of priorMaterials.values())
    if (!seenMaterials.has(old.source.externalId)) materials.push(old);
  next.courses = [
    ...next.courses.filter((c) => c.source.provider !== "school"),
    ...courses,
  ];
  next.memberships = [
    ...next.memberships.filter((m) => m.evidence.provider !== "school"),
    ...memberships,
  ];
  next.assignments = [
    ...next.assignments.filter((a) => a.source.provider !== "school"),
    ...assignments,
  ];
  next.materials = [
    ...next.materials.filter((m) => m.source.provider !== "school"),
    ...materials,
  ];
  return next;
}
