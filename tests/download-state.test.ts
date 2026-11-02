import { expect, test } from "bun:test";
import { formatBytes, mergeSnapshot, mergeTask, type DownloadTask } from "../src/features/downloads/api";
import { boundsRegion, parseRegion, regionBounds } from "../src/features/map/region";

const task = (revision: number, status = "Downloading"): DownloadTask => ({
  id: "a", file: { name: "a.zip", url: "https://example.com/a", size: 100 }, product: "SLC",
  save_dir: "D:/SAR", status, downloaded_bytes: 25, total_bytes: 100, speed_bps: 10, verified_bytes: 0, error: null, revision,
});
test("late progress and snapshots cannot overwrite newer task state", () => {
  const complete = task(8, "Completed");
  expect(mergeTask([complete], task(7))[0].status).toBe("Completed");
  expect(mergeSnapshot([complete], [task(6)])[0].status).toBe("Completed");
  expect(mergeSnapshot([task(1)], [complete])[0].status).toBe("Completed");
  expect(mergeSnapshot([complete], [])).toEqual([]);
});
test("bounds and GeoJSON imports retain polygon holes and all features", () => {
  const polygon = { type: "Polygon", coordinates: [[[0, 0], [10, 0], [10, 10], [0, 0]], [[2, 2], [2, 4], [4, 2], [2, 2]]] };
  expect(parseRegion(JSON.stringify(polygon)).coordinates).toEqual(polygon.coordinates);
  const multi = parseRegion(JSON.stringify({ type: "FeatureCollection", features: [
    { type: "Feature", properties: {}, geometry: polygon },
    { type: "Feature", properties: {}, geometry: boundsRegion([20, 20, 30, 30]) },
  ] }));
  expect(multi.type).toBe("MultiPolygon");
  expect(regionBounds(multi)).toEqual([0, 0, 30, 30]);
  expect(() => parseRegion('{"type":"Point","coordinates":[1,2]}')).toThrow();
  expect(() => parseRegion('{"type":"FeatureCollection","features":[]}')).toThrow();
  expect(boundsRegion([1, 2, 0, 3])).toBeNull();
  expect(boundsRegion([NaN, 0, 1, 1])).toBeNull();
  expect(formatBytes(1024 ** 3)).toBe("1.00 GiB");
});


import { SnapshotGate } from "../src/features/downloads/snapshotGate";
test("inflight snapshot preserves new event IDs but a later authoritative snapshot can remove them", () => {
  const gate = new SnapshotGate();
  const ticket = gate.begin();
  const added = {...task(3),id:"new"}; gate.touch(added.id);
  expect(gate.merge([added], [], ticket, new Set())).toEqual([added]);
  expect(gate.merge([added], [], gate.begin(), new Set())).toEqual([]);
});
test("out of order snapshots cannot resurrect removals or roll back newer membership", () => {
  const gate = new SnapshotGate(); const stale = gate.begin(); const latest = gate.begin();
  const one = task(8); const two = {...task(2),id:"new"};
  const current = gate.merge([one], [one,two], latest, new Set());
  expect(gate.merge(current, [task(1)], stale, new Set())).toEqual(current);
  gate.touch(one.id);
  expect(gate.merge(current, [one,two], gate.begin(), new Set([one.id]))).toEqual([two]);
});


import { finishSubmission } from "../src/features/downloads/submission";
test("created task IDs start before refresh and survive a failed display sync",async()=>{
  const calls:string[]=[];const warnings:string[]=[];
  await finishSubmission(["one","two"],async ids=>{calls.push("start:"+ids.join(","));},
    async()=>{calls.push("refresh");throw new Error("offline");},error=>warnings.push(String(error)));
  expect(calls).toEqual(["start:one,two","refresh"]);expect(warnings[0]).toContain("offline");
});
test("startup failure still refreshes created records and remains visible to caller",async()=>{
  let refreshed=false;
  await expect(finishSubmission(["one"],async()=>{throw new Error("start denied");},
    async()=>{refreshed=true;},()=>{})).rejects.toThrow("start denied");
  expect(refreshed).toBe(true);
});
