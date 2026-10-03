import type { Quarter, TermId } from "./model";

export type AcademicProfile = {
  name: string;
  /** Curriculum/study-guide cohort, independent from the current academic year. */
  admissionYear?: number;
  gradePath: Array<{ grade: string; academicYear: number }>;
  years: Array<{
    academicYear: number;
    quarterStarts: Record<Quarter, string>;
  }>;
};

const quarters: Quarter[] = ["1Q", "2Q", "3Q", "4Q"];

function isDate(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value))
    return false;
  const [year, month, day] = value.split("-").map(Number);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return (
    parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day
  );
}

export function validateAcademicProfile(value: unknown): AcademicProfile {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Academic profile must be an object");
  const input = value as Record<string, unknown>;
  if (typeof input.name !== "string" || !input.name.trim())
    throw new Error("Academic profile needs a name");
  if (
    input.admissionYear !== undefined &&
    (!Number.isInteger(input.admissionYear) || Number(input.admissionYear) < 2000)
  )
    throw new Error("Academic profile has an invalid admission year");
  if (!Array.isArray(input.gradePath) || !input.gradePath.length)
    throw new Error("Academic profile needs at least one grade path entry");
  const gradePath = input.gradePath.map((raw, index) => {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new Error(`Grade path entry ${index + 1} must be an object`);
    const item = raw as Record<string, unknown>;
    if (typeof item.grade !== "string" || !item.grade.trim())
      throw new Error(`Grade path entry ${index + 1} needs a grade label`);
    if (
      !Number.isInteger(item.academicYear) ||
      Number(item.academicYear) < 1900
    )
      throw new Error(
        `Grade path entry ${index + 1} has an invalid academic year`,
      );
    return {
      grade: item.grade.trim(),
      academicYear: Number(item.academicYear),
    };
  });
  if (new Set(gradePath.map((item) => item.grade)).size !== gradePath.length)
    throw new Error("Grade labels must be unique");
  if (!Array.isArray(input.years) || !input.years.length)
    throw new Error("Academic profile needs at least one configured year");
  const years = input.years.map((raw, index) => {
    if (!raw || typeof raw !== "object" || Array.isArray(raw))
      throw new Error(`Configured year ${index + 1} must be an object`);
    const item = raw as Record<string, unknown>;
    if (
      !Number.isInteger(item.academicYear) ||
      Number(item.academicYear) < 1900
    )
      throw new Error(
        `Configured year ${index + 1} has an invalid academic year`,
      );
    if (
      !item.quarterStarts ||
      typeof item.quarterStarts !== "object" ||
      Array.isArray(item.quarterStarts)
    )
      throw new Error(`Configured year ${index + 1} needs quarter starts`);
    const starts = item.quarterStarts as Record<string, unknown>;
    const values = quarters.map((quarter) => starts[quarter]);
    if (!values.every(isDate))
      throw new Error(
        `Configured year ${index + 1} needs valid starts for 1Q–4Q`,
      );
    if (
      values.some(
        (date, position) => position > 0 && date! <= values[position - 1]!,
      )
    )
      throw new Error(
        `Configured year ${index + 1} quarter starts must be chronological`,
      );
    return {
      academicYear: Number(item.academicYear),
      quarterStarts: Object.fromEntries(
        quarters.map((quarter) => [quarter, String(starts[quarter])]),
      ) as Record<Quarter, string>,
    };
  });
  if (new Set(years.map((item) => item.academicYear)).size !== years.length)
    throw new Error("Configured academic years must be unique");
  const configuredYears = new Set(years.map((item) => item.academicYear));
  if (gradePath.some((item) => !configuredYears.has(item.academicYear)))
    throw new Error("Every grade path year must have quarter starts");
  return {
    name: input.name.trim(),
    admissionYear:
      input.admissionYear === undefined ? undefined : Number(input.admissionYear),
    gradePath,
    years: years.sort((a, b) => a.academicYear - b.academicYear),
  };
}

export function academicTermForDate(
  date: Date,
  profile: AcademicProfile,
): TermId | undefined {
  if (Number.isNaN(date.getTime())) return undefined;
  const iso = new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Tokyo",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(date);
  const starts = profile.years.flatMap((year) =>
    quarters.map((quarter) => ({
      date: year.quarterStarts[quarter],
      term: `${year.academicYear}-${quarter}` as TermId,
    })),
  );
  return starts
    .filter((start) => start.date <= iso)
    .sort((a, b) => b.date.localeCompare(a.date))[0]?.term;
}
