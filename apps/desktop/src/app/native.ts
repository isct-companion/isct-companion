import { getDesktopBridge, type SchoolSite } from "../platform/desktop-bridge";
import { createSchoolWebExit } from "../platform/school-web-exit";
import { t, type MessageKey } from "../i18n";
import { errorText } from "./shared";
export type CustomizationSite = "portal" | "lms" | "syllabus";
export const official: Record<
  SchoolSite,
  { url: string; name: MessageKey; customization: CustomizationSite }
> = {
  "science-portal": {
    url: "https://isct.ex-tic.com/idm/user/portal/",
    name: "web.site.sciencePortal",
    customization: "portal",
  },
  "science-lms": {
    url: "https://lms.s.isct.ac.jp/2025/",
    name: "web.site.scienceLms",
    customization: "lms",
  },
  "tokyo-portal": {
    url: "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja",
    name: "webRouter.tokyoTechPortal",
    customization: "portal",
  },
  "tokyo-mail": {
    url: "https://mailv4.m.titech.ac.jp/cgi-bin/loginweb",
    name: "web.site.tokyoMail",
    customization: "portal",
  },
  kyomu: {
    url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
    name: "web.site.kyomu",
    customization: "portal",
  },
  tsubame: {
    url: "https://portal.t4.gsic.titech.ac.jp/ptl/ga/sso/",
    name: "web.site.tsubame",
    customization: "portal",
  },
  library: {
    url: "https://request.libra.titech.ac.jp/portal/user.php?plang=jpn",
    name: "web.site.library",
    customization: "portal",
  },
  syllabus: {
    url: "https://syllabus.s.isct.ac.jp/",
    name: "common.syllabus",
    customization: "syllabus",
  },
  "study-guide": {
    url: "https://www.titech.ac.jp/student/students/life/resources",
    name: "web.site.studyGuide",
    customization: "portal",
  },
};
export const customizationNames: Record<CustomizationSite, MessageKey> = {
  portal: "app.schoolPortals",
  lms: "web.site.scienceLms",
  syllabus: "common.syllabus",
};
export const desktopBridge = getDesktopBridge();
/**
 * Saves an export through the system Save dialog (the webview cancels
 * `<a download>`) and reports the outcome. Cancelling the dialog stays quiet.
 */
export async function saveExportFile(
  name: string,
  contents: () => string | Promise<string>,
  notify: (message: string) => void,
) {
  try {
    if (!desktopBridge) throw new Error(t("app.savingFilesNeedsTheDesktop"));
    if (await desktopBridge.saveFile(name, await contents()))
      notify(t("app.savedName", { name }));
  } catch (error) {
    notify(
      t("settings.main.exportFailedReason", {
        reason: errorText(error) ?? t("common.unknownError"),
      }),
    );
  }
}
export const schoolWebExit = createSchoolWebExit(desktopBridge);
export const hideNativeSchoolWeb = () => schoolWebExit.close();
let schoolViewOwner: symbol | undefined;
export const claimSchoolView = (owner: symbol) => {
  schoolViewOwner = owner;
};
export const releaseSchoolView = (owner: symbol) => {
  if (schoolViewOwner !== owner) return Promise.resolve();
  schoolViewOwner = undefined;
  return hideNativeSchoolWeb();
};
