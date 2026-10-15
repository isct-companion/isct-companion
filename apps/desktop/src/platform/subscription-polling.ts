import type { Subscription } from "../lib/model";

export const PUBLIC_DATA_INDEX =
  "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
export const REFERENCE_KINDS = [
  "calendar",
  "departments",
  "important-dates",
  "exams",
] as const satisfies readonly Subscription["kind"][];

type NetworkInformation = {
  type?: string;
  saveData?: boolean;
  metered?: boolean;
  addEventListener?: (type: "change", listener: () => void) => void;
  removeEventListener?: (type: "change", listener: () => void) => void;
};

type SubscriptionNetwork = "offline" | "wifi" | "cellular";

function networkInformation(): NetworkInformation | undefined {
  return (navigator as Navigator & { connection?: NetworkInformation })
    .connection;
}

/** Unknown desktop links are treated as unmetered; unknown mobile links pause. */
export function subscriptionNetwork(
  platform: "native-desktop" | "native-mobile" | "browser-preview",
): SubscriptionNetwork {
  if (!navigator.onLine) return "offline";
  const connection = networkInformation();
  if (
    connection?.type === "cellular" ||
    connection?.saveData === true ||
    connection?.metered === true
  )
    return "cellular";
  if (["wifi", "ethernet"].includes(connection?.type ?? "")) return "wifi";
  return platform === "native-mobile" ? "cellular" : "wifi";
}
