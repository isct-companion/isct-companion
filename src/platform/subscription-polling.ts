import type { Subscription, Workspace } from "../core/model";
import { isCivilDate, parseIsoTimestamp } from "../core/civil-date";

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

export function subscriptionPollDue(data: Workspace, now = Date.now()) {
  // Metadata may outlive the reference tables after a restore or interrupted
  // write. In that case the cache must be repaired immediately, even when a
  // successful check for another source made the global timestamp look fresh.
  if (
    REFERENCE_KINDS.some((kind) => {
      const row = subscriptionRow(data, kind);
      if (row?.enabled === false) return false;
      const year = subscriptionYear(data, kind);
      return (
        !!year &&
        (!row ||
          row.year !== year ||
          !parseIsoTimestamp(row.fetchedAt) ||
          !!row.lastError ||
          !referenceCachePresent(data, kind, year))
      );
    })
  )
    return true;
  const last = data.settings.subscriptionRefresh.lastCheckedAt;
  const checked = parseIsoTimestamp(last);
  return !checked || now - checked.getTime() >= SUBSCRIPTION_POLL_MS;
}

export function subscriptionYear(data: Workspace, kind: Subscription["kind"]) {
  return kind === "departments"
    ? data.academicProfile.admissionYear
    : Number(data.selectedTerm.slice(0, 4));
}

function subscriptionRow(data: Workspace, kind: Subscription["kind"]) {
  return data.subscriptions.find((row) => row.kind === kind && !row.deletedAt);
}

/** Whether locally materialized data can satisfy this subscription. */
export function referenceCachePresent(
  data: Workspace,
  kind: Subscription["kind"],
  year: number,
) {
  if (kind !== "calendar") return true;
  const boundaries = new Set(
    data.academicCalendar
      .filter(
        (row) =>
          !row.deletedAt &&
          !row.missingSince &&
          row.academicYear === year &&
          isCivilDate(row.date) &&
          (row.type === "quarter-start" || row.type === "quarter-end"),
      )
      .map((row) => `${row.type}:${row.quarter}`),
  );
  return ["1Q", "2Q", "3Q", "4Q"].every(
    (quarter) =>
      boundaries.has(`quarter-start:${quarter}`) &&
      boundaries.has(`quarter-end:${quarter}`),
  );
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
  year: number;
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
    const row = subscriptionRow(data, kind);
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
      // A new admission or academic year selects another dataset from the
      // same index, so the index hash alone cannot prove it is current.
      const sameYear = row?.year === year;
      const hasCache = referenceCachePresent(data, kind, year);
      if (
        sameYear &&
        row?.indexHash === indexHash &&
        parseIsoTimestamp(row.fetchedAt) &&
        !row.lastError &&
        hasCache
      ) {
        results.push({ kind, url, year, indexHash, skipped: true });
        continue;
      }
      results.push({
        kind,
        url,
        year,
        indexHash,
        result: await refresh(
          kind,
          url,
          year,
          indexHash,
          sameYear && hasCache ? row?.remoteHash : undefined,
        ),
      });
    } catch (error) {
      results.push({
        kind,
        url,
        year,
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
      row.year = item.year;
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
