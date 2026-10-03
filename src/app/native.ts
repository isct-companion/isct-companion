import { getDesktopBridge, type SchoolSite } from "../platform/desktop-bridge";
import { createSchoolWebExit } from "../platform/school-web-exit";
export type CustomizationSite = "portal" | "lms" | "syllabus";
export const official: Record<
  SchoolSite,
  { url: string; name: string; customization: CustomizationSite }
> = {
  "science-portal": {
    url: "https://isct.ex-tic.com/idm/user/portal/",
    name: "Science Tokyo Portal",
    customization: "portal",
  },
  "science-lms": {
    url: "https://lms.s.isct.ac.jp/2025/",
    name: "Science Tokyo LMS",
    customization: "lms",
  },
  "tokyo-portal": {
    url: "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?lang=ja",
    name: "Tokyo Tech Portal",
    customization: "portal",
  },
  "tokyo-mail": {
    url: "https://mailv4.m.titech.ac.jp/cgi-bin/loginweb",
    name: "Tokyo Tech Mail",
    customization: "portal",
  },
  kyomu: {
    url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
    name: "教務 Web システム",
    customization: "portal",
  },
  tsubame: {
    url: "https://portal.t4.gsic.titech.ac.jp/ptl/ga/sso/",
    name: "TSUBAME",
    customization: "portal",
  },
  library: {
    url: "https://request.libra.titech.ac.jp/portal/user.php?plang=jpn",
    name: "図書館サービス",
    customization: "portal",
  },
  syllabus: {
    url: "https://syllabus.s.isct.ac.jp/",
    name: "Syllabus",
    customization: "syllabus",
  },
};
export const customizationNames: Record<CustomizationSite, string> = {
  portal: "School portals",
  lms: "Science Tokyo LMS",
  syllabus: "Syllabus",
};
export const desktopBridge = getDesktopBridge();
export const schoolWebExit = createSchoolWebExit(desktopBridge);
export const hideNativeSchoolWeb = () => schoolWebExit.close();
