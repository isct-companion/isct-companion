import { browserCacheStorage, createCache } from "../cache";
/** Public syllabus text only. No LMS state, raw HTML, or downloaded resources. */
export type SyllabusContent = {
  schemaVersion: 1;
  sourceUrl: string;
  sourceLanguage: string;
  fetchedAt: string;
  sourceUpdated: string | null;
  metadata: { label: string; value: string }[];
  sections: {
    key: string;
    heading: string;
    blocks: {
      kind: "paragraph" | "listItem" | "subheading" | "tableRow";
      text: string;
      cells?: string[];
    }[];
    links: { label: string; url: string }[];
  }[];
  lecturePlan: { label: string; topic: string; objective: string }[];
  competencies: { label: string; selected: boolean }[];
};
const cacheKey = "stws:syllabus-content:v1";
export function syllabusIdentity(value: string): string | undefined {
  try {
    const url = new URL(value);
    if (
      url.protocol !== "https:" ||
      url.hostname !== "syllabus.s.isct.ac.jp" ||
      url.port ||
      url.username ||
      url.password ||
      !/^\/courses\/\d{4}\/[^?#]+\/[^/]+$/.test(url.pathname)
    )
      return;
    const language = url.searchParams.get("hl") ?? "ja";
    if (!["en", "ja"].includes(language)) return;
    return `${url.origin}${url.pathname}?hl=${language}`;
  } catch {
    return;
  }
}
export function validSyllabusContent(value: unknown): value is SyllabusContent {
  if (!value || typeof value !== "object") return false;
  const c = value as SyllabusContent;
  const str = (v: unknown) => typeof v === "string" && v.length <= 40000;
  const rows = <T>(v: T[], max: number, check: (row: T) => boolean) =>
    Array.isArray(v) &&
    v.length <= max &&
    v.every((row) => row != null && check(row));
  try {
    return (
      JSON.stringify(c).length <= 400000 &&
      c.schemaVersion === 1 &&
      !!syllabusIdentity(c.sourceUrl) &&
      ["en", "ja"].includes(c.sourceLanguage) &&
      new URL(syllabusIdentity(c.sourceUrl)!).searchParams.get("hl") ===
        c.sourceLanguage &&
      (c.metadata?.length > 0 ||
        c.sections?.some(
          (s) => s.blocks?.some((b) => b.text?.trim()) || s.links?.length > 0,
        ) ||
        c.lecturePlan?.length > 0) &&
      Number.isFinite(Date.parse(c.fetchedAt)) &&
      (c.sourceUpdated === null || str(c.sourceUpdated)) &&
      rows(c.metadata, 80, (r) => str(r.label) && str(r.value)) &&
      rows(
        c.sections,
        80,
        (s) =>
          str(s.key) &&
          str(s.heading) &&
          rows(
            s.blocks,
            300,
            (b) =>
              ["paragraph", "listItem", "subheading", "tableRow"].includes(
                b.kind,
              ) &&
              str(b.text) &&
              (b.cells === undefined || rows(b.cells, 30, str)),
          ) &&
          rows(
            s.links,
            100,
            (l) =>
              str(l.label) &&
              str(l.url) &&
              /^https?:$/.test(new URL(l.url).protocol) &&
              !new URL(l.url).username &&
              !new URL(l.url).password,
          ),
      ) &&
      rows(
        c.lecturePlan,
        150,
        (r) => str(r.label) && str(r.topic) && str(r.objective),
      ) &&
      rows(
        c.competencies,
        80,
        (r) => str(r.label) && typeof r.selected === "boolean",
      )
    );
  } catch {
    return false;
  }
}
const contentCache = createCache<SyllabusContent>({
  namespace: "syllabus-content",
  version: 1,
  ttlMs: 24 * 60 * 60 * 1000,
  maxEntries: 30,
  maxChars: 2500000,
  validate: validSyllabusContent,
  storage: browserCacheStorage,
});
function migrateLegacyContent() {
  try {
    const raw = localStorage.getItem(cacheKey);
    if (!raw || raw.length > 3000000) return;
    const values: unknown = JSON.parse(raw);
    if (Array.isArray(values))
      for (const content of values
        .filter(validSyllabusContent)
        .slice(0, 30)
        .reverse()) {
        const identity = syllabusIdentity(content.sourceUrl)!;
        if (!contentCache.read(identity))
          contentCache.set(identity, content, Date.parse(content.fetchedAt));
      }
    localStorage.removeItem(cacheKey);
  } catch {
    /* An invalid legacy cache cannot block course editing. */
  }
}
export const readSyllabusContent = (url: string) => {
  migrateLegacyContent();
  const identity = syllabusIdentity(url);
  return identity ? contentCache.read(identity)?.value : undefined;
};
export function cacheSyllabusContent(content: SyllabusContent) {
  if (!validSyllabusContent(content)) return false;
  migrateLegacyContent();
  return contentCache.set(
    syllabusIdentity(content.sourceUrl)!,
    content,
    Date.parse(content.fetchedAt),
  );
}
