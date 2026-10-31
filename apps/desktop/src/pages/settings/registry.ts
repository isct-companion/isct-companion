import type { Platform, Section } from "../../app/shared";
import { nativeOs } from "../../platform/desktop-bridge";
import type { MessageKey } from "../../i18n";

/**
 * Settings follow the layered model of mature system settings: everyday
 * pages first; Advanced for capable users who never touch code (data
 * updates, automation, AI agents); and code-level tools behind the Developer
 * options switch. About closes the list. Search reaches every layer.
 */
export type SectionTier = "basic" | "advanced" | "developer" | "about";

export type SectionDefinition = {
  id: Section;
  label: MessageKey;
  tier: SectionTier;
};

const SECTIONS: SectionDefinition[] = [
  { id: "accounts", label: "settings.main.schoolAccounts", tier: "basic" },
  { id: "wifi", label: "settings.campusWifi.campusWiFi", tier: "basic" },
  { id: "study", label: "settings.academicCalendar.study", tier: "basic" },
  { id: "notifications", label: "common.notifications", tier: "basic" },
  { id: "appearance", label: "settings.appearance.appearance", tier: "basic" },
  { id: "files", label: "settings.files.courseFiles", tier: "basic" },
  { id: "backup", label: "settings.main.backup", tier: "basic" },
  { id: "sources", label: "settings.main.dataUpdates", tier: "advanced" },
  { id: "agent", label: "settings.agent.aiAgentAccess", tier: "advanced" },
  {
    id: "developer",
    label: "settings.developer.developerOptions",
    tier: "advanced",
  },
  {
    id: "extensions",
    label: "settings.extensions.siteStylesScripts",
    tier: "developer",
  },
  {
    id: "diagnostics",
    label: "settings.main.applicationLogs",
    tier: "developer",
  },
  { id: "about", label: "settings.main.about", tier: "about" },
];

/** Pages reached from another page rather than listed in the sidebar. */
const NESTED: Partial<Record<Section, { label: MessageKey; parent: Section }>> =
  {
    calendar: {
      label: "settings.academicCalendar.academicCalendar",
      parent: "study",
    },
    // Scheduled tasks share the Data updates page: both keep data fresh.
    automation: { label: "settings.main.dataUpdates", parent: "sources" },
  };

export type SectionContext = {
  platform: Platform;
  developer: boolean;
  extensionsAvailable: boolean;
};

/** Whether a page exists on this platform at all, regardless of the switch. */
export function sectionSupported(id: Section, context: SectionContext) {
  if (id === "files") return context.platform === "native-desktop";
  if (id === "extensions") return context.extensionsAvailable;
  // ScienceTokyo Wi-Fi sign-in is compiled only into macOS builds.
  if (id === "wifi") return nativeOs() === "macos";
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
    "common.settings"
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
  title: MessageKey;
  terms?: string;
  /** False on a platform whose page leaves this row out. */
  available?: () => boolean;
};

export const SETTING_ENTRIES: SettingEntry[] = [
  {
    section: "about",
    id: "updates",
    title: "settings.about.appUpdates",
    terms: "version update download 更新 版本",
  },
  {
    section: "about",
    id: "automatic-updates",
    title: "settings.about.automaticUpdates",
    terms: "background install 自动更新 自動更新",
  },
  {
    section: "accounts",
    id: "science",
    title: "settings.registry.scienceTokyoAccount",
    terms: "login sign in password otp portal ログイン パスワード",
  },
  {
    section: "accounts",
    id: "tokyo-tech",
    title: "settings.registry.tokyoTechAccount",
    terms: "login sign in matrix portal ログイン マトリクス",
  },
  {
    section: "wifi",
    id: "campus-wifi",
    title: "settings.campusWifi.sciencetokyoWiFi",
    terms: "wifi wi-fi wireless network internet campus 無線LAN ネットワーク",
  },
  {
    section: "study",
    id: "department",
    title: "settings.main.department",
    terms: "major 系 学科 course recommendations 推奨",
  },
  {
    section: "study",
    id: "admission-year",
    title: "settings.main.admissionYear",
    terms: "admission entry cohort guide 入学 年度 学修案内",
  },
  {
    section: "study",
    id: "study-stages",
    title: "settings.studyProfile.degreeProgramme",
    terms:
      "undergraduate master professional doctoral transfer grade 学士 修士 専門職 博士 編入 年次",
  },
  {
    section: "notifications",
    id: "permission",
    title: "settings.notification.allowNotifications",
    terms: "permission 許可",
  },
  {
    section: "notifications",
    id: "classes",
    title: "settings.notification.classReminders",
    terms: "lecture 授業 リマインダー",
  },
  {
    section: "notifications",
    id: "class-lead",
    title: "settings.main.remindMe",
    terms: "minutes before lead time 分前",
  },
  {
    section: "notifications",
    id: "assignments",
    title: "settings.notification.assignmentReminders",
    terms: "deadline due 課題 締切",
  },
  {
    section: "notifications",
    id: "assignment-lead",
    title: "settings.notification.assignmentReminderTimes",
    terms: "deadline minutes hours days before 提前 作业 提醒 締切",
  },
  {
    section: "calendar",
    id: "days",
    title: "settings.registry.holidaysAndMakeUpDays",
    terms: "holiday replacement no class 休講 振替 祝日",
  },
  {
    section: "calendar",
    id: "quarters",
    title: "settings.registry.quarterDates",
    terms: "term semester クォーター 学期",
  },
  {
    section: "calendar",
    id: "important-dates",
    title: "today.importantDates",
    terms: "events festival 行事 重要",
  },
  {
    section: "appearance",
    id: "language",
    title: "common.language",
    terms: "english japanese 日本語 英語 locale",
  },
  {
    section: "appearance",
    id: "content-language",
    title: "settings.main.schoolContentLanguage",
    terms: "course name syllabus lms 授業名 シラバス 言語",
  },
  {
    section: "appearance",
    id: "appearance-mode",
    title: "settings.appearance.lightAndDark",
    terms: "dark light mode ダーク ライト",
  },
  {
    section: "appearance",
    id: "theme",
    title: "settings.main.theme",
    terms: "colour color background warm cool paper mist moss 背景 色 テーマ",
  },
  {
    section: "appearance",
    id: "text-size",
    title: "common.textSize",
    terms: "font zoom 文字",
  },
  {
    section: "appearance",
    id: "close-window",
    title: "settings.appearance.closeWindow.title",
    terms: "close quit tray dock background 关闭 退出 托盘 閉じる 終了 トレイ",
    // Only the macOS and Windows shells keep running after the window closes.
    available: () => nativeOs() === "macos" || nativeOs() === "windows",
  },
  {
    section: "appearance",
    id: "background",
    title: "settings.appearance.backgroundImage",
    terms: "wallpaper 背景",
  },
  {
    section: "appearance",
    id: "density",
    title: "settings.appearance.density",
    terms: "compact spacing 密度",
  },
  {
    section: "files",
    id: "folder",
    title: "settings.courseStorage.courseFolder",
    terms: "download location path フォルダ 保存先",
  },
  {
    section: "files",
    id: "auto-download",
    title: "settings.files.downloadNewFilesAutomatically",
    terms: "cache size 自動 ダウンロード",
  },
  {
    section: "files",
    id: "workspaces",
    title: "settings.files.assignmentWorkspaces",
    terms: "workspace clean delete vscode submit 課題 作業 フォルダ 削除",
  },
  {
    section: "appearance",
    id: "bilingual",
    title: "settings.appearance.showOneLanguageInBilingual",
    terms: "translation 二言語",
  },
  {
    section: "backup",
    id: "cloud",
    title: "settings.backup.sync",
    terms: "cloud backup sync icloud google drive クラウド 同期 iCloud",
  },
  {
    section: "backup",
    id: "file",
    title: "settings.registry.exportOrImportABackup",
    terms: "json export import 書き出し 読み込み",
  },
  {
    section: "backup",
    id: "clear-caches",
    title: "settings.reset.caches.title",
    terms:
      "cache resync refetch reimport school data clear キャッシュ 再同期 再取得 缓存 重新同步",
  },
  {
    section: "backup",
    id: "full-reset",
    title: "settings.reset.full.title",
    terms:
      "reset erase delete wipe first launch 初期化 リセット 消去 重置 清除",
  },
  {
    section: "sources",
    id: "calendar",
    title: "settings.registry.officialDataUpdates",
    terms:
      "subscription github calendar guide exams alternating physics 購読 試験 隔週 物理",
  },
  {
    section: "sources",
    id: "cellular",
    title: "settings.main.updateOnCellularData",
    terms: "mobile data metered wifi モバイル通信",
  },
  {
    section: "automation",
    id: "tasks",
    title: "settings.automation.scheduledTasks",
    terms: "background refresh export cron 自動 定期",
  },
  {
    section: "developer",
    id: "developer-mode",
    title: "settings.developer.developerOptions",
    terms: "advanced debug 開発者",
  },
  {
    section: "developer",
    id: "profile",
    title: "settings.developer.academicProfile",
    terms:
      "quarter starts class times teaching periods timetable json 校历 時限 节次 上课时间",
  },
  {
    section: "developer",
    id: "sources",
    title: "settings.developer.dataSourceURLs",
    terms: "index url github subscription",
  },
  {
    section: "agent",
    id: "mcp",
    title: "settings.agent.aiAgentAccess",
    terms: "mcp claude agent server エージェント",
  },
  {
    section: "agent",
    id: "lms-writes",
    title: "settings.agent.allowLmsWrites",
    terms: "lms submit submission hand in 提出 提交",
  },
  {
    section: "extensions",
    id: "sites",
    title: "settings.extensions.siteStylesScripts",
    terms: "css javascript js userscript customize カスタマイズ",
  },
  {
    section: "diagnostics",
    id: "capabilities",
    title: "settings.registry.diagnostics",
    terms: "logs activity debug capability ログ",
  },
  {
    section: "about",
    id: "guide",
    title: "settings.main.showWelcomeAndTipsAgain",
    terms:
      "tutorial onboarding help hints guide チュートリアル ヒント 教程 提示",
  },
  {
    section: "about",
    id: "version",
    title: "settings.about.version",
    terms: "about info バージョン 情報",
  },
];
