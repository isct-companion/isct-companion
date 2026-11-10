export type OpenedContent = {
  id: string;
  title: string;
  kind: "material" | "official" | "link" | "note" | "syllabus" | "announcement";
  section?: string;
  url?: string;
};
export type ReadingState = {
  version: 3;
  expanded: string[];
  scroll: number;
  opened?: OpenedContent;
  anchor?: { id: string; ratio: number };
  /** The lesson shown alone in the outline, by section id. */
  lesson?: number;
};
const reading = new Map<string, ReadingState>();

export function loadReading(key: string): ReadingState | undefined {
  return (
    reading.get(key) ??
    (() => {
      try {
        const value = JSON.parse(
          sessionStorage.getItem(`lms-reading:${key}`) || "null",
        );
        return value && value.version === 3 && Array.isArray(value.expanded)
          ? (value as ReadingState)
          : undefined;
      } catch {
        return undefined;
      }
    })()
  );
}

export function persistReading(
  key: string,
  deviceId: string,
  state: ReadingState,
) {
  reading.delete(key);
  reading.set(key, state);
  if (reading.size > 8) reading.delete(reading.keys().next().value!);
  try {
    sessionStorage.setItem(`lms-reading:${key}`, JSON.stringify(state));
  } catch {
    /* Reading remains available in memory. */
  }
  try {
    const indexKey = `lms-reading-index:${deviceId}`;
    const stored: unknown = JSON.parse(
      sessionStorage.getItem(indexKey) || "[]",
    );
    const index = [
      ...(Array.isArray(stored)
        ? stored.filter((k) => typeof k === "string" && k !== key)
        : []),
      key,
    ];
    index
      .slice(0, -8)
      .forEach((k) => sessionStorage.removeItem(`lms-reading:${k}`));
    sessionStorage.setItem(indexKey, JSON.stringify(index.slice(-8)));
  } catch {
    /* Persistence is optional. */
  }
}
