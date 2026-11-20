import { workspaceActions } from "./workspace";

/** This device's on-demand choice, which the next launch reads. */
export const saveVpnOnDemand = (enabled: boolean) =>
  workspaceActions.updateSettings((draft) => {
    draft.settings.campusVpnOnDemand = enabled;
    return draft;
  });

/** This device's publisher list; `undefined` stores the default. */
export const saveVpnPublishers = (list: string[] | undefined) =>
  workspaceActions.updateSettings((draft) => {
    draft.settings.campusVpnPublishers = list;
    return draft;
  });
