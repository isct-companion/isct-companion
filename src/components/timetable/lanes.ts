/**
 * Side-by-side lanes for overlapping blocks in one day column, so a
 * conflicting course is never hidden under another one.
 */
export function blockLanes<T extends { key: string; startTime: string; endTime: string }>(
  blocks: T[],
): Map<string, { lane: number; lanes: number }> {
  const sorted = [...blocks].sort(
    (a, b) => a.startTime.localeCompare(b.startTime) || b.endTime.localeCompare(a.endTime),
  );
  const result = new Map<string, { lane: number; lanes: number }>();
  let cluster: { key: string; lane: number }[] = [];
  let clusterEnd = "";
  let laneEnds: string[] = [];
  const close = () => {
    for (const item of cluster) result.set(item.key, { lane: item.lane, lanes: laneEnds.length });
    cluster = [];
    laneEnds = [];
  };
  for (const block of sorted) {
    // A block starting after everything so far ended begins a new cluster.
    if (cluster.length && block.startTime >= clusterEnd) close();
    let lane = laneEnds.findIndex((end) => end <= block.startTime);
    if (lane < 0) lane = laneEnds.push(block.endTime) - 1;
    else laneEnds[lane] = block.endTime;
    cluster.push({ key: block.key, lane });
    if (cluster.length === 1 || block.endTime > clusterEnd) clusterEnd = block.endTime;
  }
  close();
  return result;
}
