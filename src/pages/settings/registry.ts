import type { Platform, Section } from "../../app/shared";

/**
 * Settings follow the layered model of mature system settings: everyday
 * pages first; Advanced for capable users who never touch code (data
 * updates, automation, AI agents); and code-level tools behind the Developer
 * options switch. About closes the list. Search reaches every layer.
 */
export type SectionTier = "basic" | "advanced" | "developer" | "about";

export type SectionDefinition = {
  id: Section;
  label: string;
  tier: SectionTier;
};

const SECTIONS: SectionDefinition[] = [
  { id: "accounts", label: "School accounts", tier: "basic" },
  { id: "study", label: "Study", tier: "basic" },
  { id: "notifications", label: "Notifications", tier: "basic" },
  { id: "appearance", label: "Appearance", tier: "basic" },
  { id: "files", label: "Course files", tier: "basic" },
  { id: "backup", label: "Backup", tier: "basic" },
  { id: "sources", label: "Data updates", tier: "advanced" },
  { id: "automation", label: "Automation", tier: "advanced" },
  { id: "agent", label: "AI agent access", tier: "advanced" },
  { id: "developer", label: "Developer options", tier: "advanced" },
  { id: "extensions", label: "Site styles & scripts", tier: "developer" },
  { id: "diagnostics", label: "Diagnostics", tier: "developer" },
  { id: "about", label: "About", tier: "about" },
];

/** Pages reached from another page rather than listed in the sidebar. */
const NESTED: Partial<Record<Section, { label: string; parent: Section }>> = {
  calendar: { label: "Academic calendar", parent: "study" },
};

export type SectionContext = {
  platform: Platform;
  developer: boolean;
  extensionsAvailable: boolean;
};

/**
 * Whether a page exists on this platform at all, regardless of the switch.
 * Phone layouts and their feature set are designed separately; the
 * native-mobile target (iPhone and iPad alike) keeps its earlier pages.
 */
export function sectionSupported(id: Section, context: SectionContext) {
  const desktop = context.platform === "native-desktop";
  const mobile = context.platform === "native-mobile";
  if (id === "files") return desktop;
  if (id === "extensions") return context.extensionsAvailable;
  if (
    mobile &&
    (id === "calendar" ||
      id === "developer" ||
      id === "agent" ||
      id === "diagnostics")
  )
    return false;
  return true;
}

export function visibleSections(context: SectionContext) {
  return SECTIONS.filter(
    (section) =>
      sectionSupported(section.id, context) &&
      (section.tier !== "developer" || context.developer),
  );
}

export function sectionLabel(id: Section) {
  return (
    SECTIONS.find((section) => section.id === id)?.label ??
    NESTED[id]?.label ??
    id
  );
}

export function sectionTier(id: Section) {
  return SECTIONS.find((section) => section.id === id)?.tier ?? "basic";
}

/** The sidebar entry a page belongs to; nested pages highlight their parent. */
export function sectionParent(id: Section) {
  return NESTED[id]?.parent ?? id;
}

/**
 * Searchable settings. `id` matches the `Row id` on the page; `terms` are
 * extra words people might type (English and Japanese).
 */
export type SettingEntry = {
  section: Section;
  id: string;
  title: string;
  terms?: string;
};

export const SETTING_ENTRIES: SettingEntry[] = [
  {
    section: "accounts",
    id: "science",
    title: "Science Tokyo account",
    terms: "login sign in password otp portal ログイン パスワード",
  },
  {
    section: "accounts",
    id: "tokyo-tech",
    title: "Tokyo Tech account",
    terms: "login sign in matrix portal ログイン マトリクス",
  },
  {
    section: "study",
    id: "department",
    title: "Department",
    terms: "major 系 学科 course recommendations 推奨",
  },
  {
    section: "study",
    id: "admission-year",
    title: "Year you started",
    terms: "admission entry cohort guide 入学 年度 学修案内",
  },
  {
    section: "notifications",
    id: "permission",
    title: "Allow notifications",
    terms: "permission 許可",
  },
  {
    section: "notifications",
    id: "classes",
    title: "Class reminders",
    terms: "lecture 授業 リマインダー",
  },
  {
    section: "notifications",
    id: "class-lead",
    title: "Remind me",
    terms: "minutes before lead time 分前",
  },
  {
    section: "notifications",
    id: "assignments",
    title: "Assignment reminders",
    terms: "deadline due 課題 締切",
  },
  {
    section: "calendar",
    id: "days",
    title: "Holidays and make-up days",
    terms: "holiday replacement no class 休講 振替 祝日",
  },
  {
    section: "calendar",
    id: "quarters",
    title: "Quarter dates",
    terms: "term semester クォーター 学期",
  },
  {
    section: "calendar",
    id: "important-dates",
    title: "Important dates",
    terms: "events festival 行事 重要",
  },
  {
    section: "appearance",
    id: "language",
    title: "Language",
    terms: "english japanese 日本語 英語 locale",
  },
  {
    section: "appearance",
    id: "content-language",
    title: "School content language",
    terms: "course name syllabus lms 授業名 シラバス 言語",
  },
  {
    section: "appearance",
    id: "theme",
    title: "Theme",
    terms: "dark light mode ダーク ライト",
  },
  {
    section: "appearance",
    id: "text-size",
    title: "Text size",
    terms: "font zoom 文字",
  },
  {
    section: "appearance",
    id: "background",
    title: "Background image",
    terms: "wallpaper 背景",
  },
  {
    section: "appearance",
    id: "density",
    title: "Density",
    terms: "compact spacing 密度",
  },
  {
    section: "files",
    id: "folder",
    title: "Course folder",
    terms: "download location path フォルダ 保存先",
  },
  {
    section: "files",
    id: "auto-download",
    title: "Download new files automatically",
    terms: "cache size 自動 ダウンロード",
  },
  {
    section: "appearance",
    id: "bilingual",
    title: "Show one language in bilingual LMS",
    terms: "translation 二言語",
  },
  {
    section: "backup",
    id: "cloud",
    title: "Google Drive",
    terms: "cloud backup sync クラウド 同期",
  },
  {
    section: "backup",
    id: "file",
    title: "Export or import a backup file",
    terms: "json export import 書き出し 読み込み",
  },
  {
    section: "backup",
    id: "reset",
    title: "Erase data on this device",
    terms: "reset clear delete 初期化",
  },
  {
    section: "sources",
    id: "calendar",
    title: "Official data updates",
    terms: "subscription github calendar guide exams 購読 試験",
  },
  {
    section: "sources",
    id: "cellular",
    title: "Update on cellular data",
    terms: "mobile data metered wifi モバイル通信",
  },
  {
    section: "automation",
    id: "tasks",
    title: "Scheduled tasks",
    terms: "background refresh export cron 自動 定期",
  },
  {
    section: "developer",
    id: "developer-mode",
    title: "Developer options",
    terms: "advanced debug 開発者",
  },
  {
    section: "developer",
    id: "profile",
    title: "Academic profile",
    terms: "grade quarter starts json 学年",
  },
  {
    section: "developer",
    id: "sources",
    title: "Data source URLs",
    terms: "index url github subscription",
  },
  {
    section: "agent",
    id: "mcp",
    title: "AI agent access",
    terms: "mcp claude agent server エージェント",
  },
  {
    section: "extensions",
    id: "sites",
    title: "Site styles & scripts",
    terms: "css javascript js userscript customize カスタマイズ",
  },
  {
    section: "diagnostics",
    id: "capabilities",
    title: "Diagnostics",
    terms: "logs activity debug capability ログ",
  },
  {
    section: "about",
    id: "version",
    title: "Version",
    terms: "about info バージョン 情報",
  },
];
