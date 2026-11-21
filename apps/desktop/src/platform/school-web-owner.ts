import { desktopBridge } from "./native";
import { createSchoolWebExit } from "./school-web-exit";

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
