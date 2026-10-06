import type { Subscription, Workspace } from "../core/model";

export const PUBLIC_DATA_INDEX =
  "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
export const SUBSCRIPTION_POLL_MS = 24 * 60 * 60 * 1000;
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

export type SubscriptionNetwork = "offline" | "wifi" | "cellular";

export function networkInformation(): NetworkInformation | undefined {
  return (navigator as Navigator & { connection?: NetworkInformation }).connection;
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

export function subscriptionPollDue(data: Workspace, now = Date.now()) {
  const last = data.settings.subscriptionRefresh.lastCheckedAt;
  return !last || !Number.isFinite(Date.parse(last)) || now - Date.parse(last) >= SUBSCRIPTION_POLL_MS;
}

export function subscriptionYear(data: Workspace, kind: Subscription["kind"]) {
  return kind === "departments"
    ? data.academicProfile.admissionYear
    : Number(data.selectedTerm.slice(0, 4));
}

export type ReferenceRefreshResult = {
  changed: number;
  dataUrl: string;
  remoteHash: string;
  sourceTs: string;
  notModified: boolean;
};

export type ReferenceProbeResult = {
  indexHash: string;
};

export type PollResult = {
  kind: Subscription["kind"];
  url: string;
  indexHash?: string;
  skipped?: boolean;
  result?: ReferenceRefreshResult;
  error?: string;
};

export async function pollReferenceSubscriptions(
  data: Workspace,
  probe: (indexUrl: string) => Promise<ReferenceProbeResult>,
  refresh: (
    kind: Subscription["kind"],
    indexUrl: string,
    year: number,
    expectedIndexHash: string,
    knownHash?: string,
  ) => Promise<ReferenceRefreshResult>,
): Promise<PollResult[]> {
  const results: PollResult[] = [];
  const probes = new Map<string, Promise<ReferenceProbeResult>>();
  for (const kind of REFERENCE_KINDS) {
    const row = data.subscriptions.find(
      (item) => item.kind === kind && !item.deletedAt,
    );
    if (row?.enabled === false) continue;
    const year = subscriptionYear(data, kind);
    if (!year) continue;
    const url = row?.url.startsWith("https://") ? row.url : PUBLIC_DATA_INDEX;
    try {
      let pendingProbe = probes.get(url);
      if (!pendingProbe) {
        pendingProbe = probe(url);
        probes.set(url, pendingProbe);
      }
      const { indexHash } = await pendingProbe;
      if (row?.indexHash === indexHash) {
        results.push({ kind, url, indexHash, skipped: true });
        continue;
      }
      results.push({
        kind,
        url,
        indexHash,
        result: await refresh(kind, url, year, indexHash, row?.remoteHash),
      });
    } catch (error) {
      results.push({
        kind,
        url,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }
  return results;
}

export function recordSubscriptionPoll(
  draft: Workspace,
  results: PollResult[],
  checkedAt: string,
) {
  if (results.length > 0 && results.every((item) => !item.error))
    draft.settings.subscriptionRefresh.lastCheckedAt = checkedAt;
  for (const item of results) {
    let row = draft.subscriptions.find(
      (candidate) => candidate.kind === item.kind && !candidate.deletedAt,
    );
    if (!row) {
      row = {
        id: `subscription:public:${item.kind}`,
        kind: item.kind,
        url: item.url,
        enabled: true,
        rev: 0,
        updatedAt: checkedAt,
        updatedBy: draft.deviceId,
      };
      draft.subscriptions.push(row);
    }
    row.url = item.url;
    row.lastError = item.error;
    if (item.result) {
      row.indexHash = item.indexHash;
      row.dataUrl = item.result.dataUrl;
      row.remoteHash = item.result.remoteHash;
      row.sourceTs = item.result.sourceTs;
      row.fetchedAt = checkedAt;
    } else if (item.skipped) {
      row.indexHash = item.indexHash;
    }
    row.rev += 1;
    row.updatedAt = checkedAt;
    row.updatedBy = draft.deviceId;
  }
  return draft;
}
