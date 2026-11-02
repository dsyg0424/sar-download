import { expect, test } from "bun:test";
import { createSarSearchStore } from "../src/features/sar-download/store/useSarSearchStore";
import type { SarProduct, SarSearchEvent, SarSearchResponse } from "../src/features/sar-download/services/sarSearchApi";

const product = (id: string, kind = "GSLC"): SarProduct => ({
  id, granule: id, name: id + ".h5", product: kind, url: "https://example.com/" + id,
  size: 10, date: "2026-06-01", platform: "NISAR", polarization: "HH", direction: "ASCENDING", relativeOrbit: 1,
  geometry: { type: "Polygon", coordinates: [[[0,0],[1,0],[1,1],[0,0]]] },
});

function pendingSearch() {
  let send!: (event: SarSearchEvent) => void;
  let resolve!: (value: SarSearchResponse) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<SarSearchResponse>((yes, no) => { resolve = yes; reject = no; });
  const store = createSarSearchStore((_request, progress) => { send = progress!; return promise; });
  store.getState().update({ bounds: [-99.25,19.25,-98.9,19.6], start: "2026-06-01", end: "2026-09-30", productTypes: ["GSLC","RSLC","BURST"] });
  store.setState({ results: [product("old")], selected: ["old"], searched: true });
  const running = store.getState().search();
  return { store, running, send: (event: SarSearchEvent) => send(event), resolve, reject };
}

test("verified products appear before terminal; stable merged result preserves preview selection", async () => {
  const query = pendingSearch();
  query.send({ event: "product_complete", product: "GSLC", data: { results: [product("a")], limitReached: false } });
  expect(query.store.getState().busy).toBe(true);
  expect(query.store.getState().results.map(p => p.id)).toEqual(["a"]);
  expect(query.store.getState().searchBackup?.selected).toEqual(["old"]);
  query.store.getState().toggle("a");
  query.send({ event: "product_complete", product: "RSLC", data: { results: [product("b","RSLC")], limitReached: false } });
  expect(query.store.getState().results.map(p => p.id)).toEqual(["b","a"]);
  query.resolve({ results: [product("b","RSLC"),product("a")], limitReached: false });
  await query.running;
  expect(query.store.getState().selected).toEqual(["a"]);
  expect(query.store.getState().searchBackup).toBeNull();
  query.send({ event: "product_complete", product: "BURST", data: { results: [product("late")], limitReached: false } });
  expect(query.store.getState().results.map(p => p.id)).toEqual(["b","a"]);
});

test("a failed later product restores the complete previous results and selection", async () => {
  const query = pendingSearch();
  query.send({ event: "product_complete", product: "GSLC", data: { results: [product("a")], limitReached: false } });
  query.reject(new Error("BURST unavailable"));
  await query.running;
  expect(query.store.getState().results.map(p => p.id)).toEqual(["old"]);
  expect(query.store.getState().selected).toEqual(["old"]);
  expect(query.store.getState().error).toContain("BURST unavailable");
  expect(query.store.getState().busy).toBe(false);
});

test("new project/reset rejects old progress and completion", async () => {
  const query = pendingSearch();
  query.store.setState({ results: [product("new-project")], resetVersion: query.store.getState().resetVersion+1, searchBackup: null });
  query.send({ event: "product_complete", product: "GSLC", data: { results: [product("stale")], limitReached: false } });
  query.resolve({ results: [product("stale")], limitReached: false });
  await query.running;
  expect(query.store.getState().results.map(p => p.id)).toEqual(["new-project"]);
});

test("pages render before product completion, deduplicate, and roll back on incomplete index", async () => {
  const query=pendingSearch();
  query.send({event:"product_page",product:"GSLC",data:{results:[product("a")],limitReached:false}});
  expect(query.store.getState().results.map(item=>item.id)).toEqual(["a"]);
  expect(query.store.getState().searchProgress.GSLC.complete).toBe(false);
  query.store.getState().toggle("a");
  query.send({event:"product_page",product:"GSLC",data:{results:[product("a"),product("b")],limitReached:false}});
  expect(query.store.getState().results.length).toBe(2);
  query.send({event:"stage",product:"GSLC",stage:"batch_parse",state:"started"});
  expect(query.store.getState().searchProgress.GSLC.count).toBe(2);
  expect(query.store.getState().selected).toEqual(["a"]);
  query.reject(new Error("CMR index incomplete"));await query.running;
  expect(query.store.getState().results.map(item=>item.id)).toEqual(["old"]);
  expect(query.store.getState().selected).toEqual(["old"]);
});


test("cancellation restores previous results and rejects delayed progress and success", async () => {
  let send!: (event: SarSearchEvent) => void; let finish!: (response:SarSearchResponse)=>void;
  let id = ""; const cancelled:string[]=[];
  const store=createSarSearchStore((_request,progress,requestId)=>{
    send=progress!;id=requestId!;return new Promise(resolve=>{finish=resolve;});
  }, async requestId=>{cancelled.push(requestId);});
  store.getState().update({bounds:[0,0,1,1],start:"2026-06-01",end:"2026-06-02",productTypes:["GSLC"]});
  store.setState({results:[product("old")],selected:["old"],searched:true});
  const running=store.getState().search();
  send({event:"product_page",product:"GSLC",data:{results:[product("partial")],limitReached:false}});
  await store.getState().cancel(); await store.getState().cancel();
  expect(cancelled).toEqual([id]); expect(store.getState().busy).toBe(true);
  send({event:"product_page",product:"GSLC",data:{results:[product("late")],limitReached:false}});
  expect(store.getState().results.map(p=>p.id)).toEqual(["partial"]);
  finish({results:[product("late")],limitReached:false}); await running;
  expect(store.getState().selected).toEqual(["old"]);
  expect(store.getState().results.map(p=>p.id)).toEqual(["old"]);
  expect(store.getState().busy).toBe(false);expect(store.getState().cancelling).toBe(false);
  expect(store.getState().error).toContain("已取消");
});
test("failed cancellation keeps the active query usable and reports failure",async()=>{
  let finish!:(value:SarSearchResponse)=>void;
  const store=createSarSearchStore(()=>new Promise(resolve=>{finish=resolve;}),async()=>{throw new Error("IPC unavailable");});
  store.getState().update({bounds:[0,0,1,1],start:"2026-06-01",end:"2026-06-02",productTypes:["GSLC"]});
  const running=store.getState().search();await store.getState().cancel();
  expect(store.getState().cancelling).toBe(false);expect(store.getState().busy).toBe(true);
  expect(store.getState().error).toContain("取消失败");
  finish({results:[product("done")],limitReached:false});await running;
  expect(store.getState().results[0].id).toBe("done");
});
