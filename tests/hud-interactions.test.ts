import { afterAll, afterEach, beforeEach, expect, test } from "bun:test";
import { JSDOM } from "jsdom";
import { act, createElement as h, useState } from "react";

const dom = new JSDOM('<!doctype html><html><body><div id="root"></div></body></html>', { url: "http://localhost/", pretendToBeVisual: true });
const originals = new Map<string, PropertyDescriptor | undefined>();
for (const name of ["window", "document", "navigator", "HTMLElement", "HTMLInputElement", "HTMLSelectElement", "HTMLOptGroupElement", "HTMLDialogElement", "Node", "Event", "MouseEvent", "KeyboardEvent", "localStorage", "requestAnimationFrame", "cancelAnimationFrame"]) {
  originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
  const value = (dom.window as any)[name]; Object.defineProperty(globalThis, name, { value: name.endsWith("AnimationFrame") ? value.bind(dom.window) : value, configurable: true });
}
(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
dom.window.HTMLElement.prototype.scrollIntoView = () => {};
dom.window.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
const { createRoot } = await import("react-dom/client");
const { DatePicker } = await import("../src/shared/components/DatePicker");
const { SarSearchForm } = await import("../src/features/sar-download/components/SarSearchForm");
const { useSarSearchStore } = await import("../src/features/sar-download/store/useSarSearchStore");
const { DownloadCenter } = await import("../src/features/downloads/DownloadCenter");
const { CleanupDialog } = await import("../src/features/downloads/CleanupDialog");
const { MapLayerControls } = await import("../src/features/map/MapLayerControls");
const { RegionSelector } = await import("../src/features/map/RegionSelector");
const { useMapStore } = await import("../src/features/map/useMapStore");
const { SettingsDialog } = await import("../src/features/downloads/SettingsDialog");
const { defaults } = await import("../src/features/downloads/api");
const { installUiInteractionPolicy } = await import("../src/shared/utils/uiInteractionPolicy");
const { useSarResults } = await import("../src/features/sar-download/hooks/useSarResults");
const { SarResultsPanel } = await import("../src/features/sar-download/components/SarResultsPanel");
import type { DownloadTask } from "../src/features/downloads/api";

let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  document.body.innerHTML = '<div id="root"></div>';
  root = createRoot(document.getElementById("root")!);
  useSarSearchStore.getState().reset();
});
afterEach(async () => {
  await act(async () => root.unmount());
  useSarSearchStore.setState({ busy: false });
  useMapStore.setState({ drawing: false });
});
afterAll(() => {
  dom.window.close();
  for (const [name, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else delete (globalThis as any)[name];
  }
  delete (globalThis as any).IS_REACT_ACT_ENVIRONMENT;
});
const button = (text: string) => [...document.querySelectorAll("button")].find((el) => el.textContent?.trim() === text)!;
const task = (id: string, status: string): DownloadTask => ({ id, file: { name: id+".h5", url: "https://example.invalid/"+id, size: 1024 }, product: "GSLC", save_dir: "X:/fixture", status, downloaded_bytes: status === "Completed" ? 1024 : 512, total_bytes: 1024, speed_bps: 0, verified_bytes: 0, error: null, revision: 1 });
const centerProps = { tasks: [] as DownloadTask[], pending: new Set<string>(), onAction: async () => {}, onOpen: () => {} };

test("cleanup previews exact files and bytes and executes only selected server ids", async()=>{
  const executed:any[]=[];
  const actions={preview:async()=>({token:"preview",totalBytes:30,items:[{id:"one",path:"E:/SAR/.sar-download/staging/one/payload.part",bytes:10,category:"未完成下载"},{id:"two",path:"C:/app/downloading/two/state.json",bytes:20,category:"失效续传记录"}]}),execute:async(token:string,ids:string[])=>{executed.push({token,ids});return {removedFiles:1,freedBytes:10,errors:[]};}};
  await act(async()=>root.render(h(CleanupDialog,{actions,onClose:()=>{}})));
  expect(executed.length).toBe(0);
  expect(document.body.textContent).toContain("30 B");
  const check=document.querySelector<HTMLInputElement>('[aria-label="清理 C:/app/downloading/two/state.json"]')!;
  await act(async()=>check.click());
  await act(async()=>button("清理所选（1）").click());
  expect(executed).toEqual([{token:"preview",ids:["one"]}]);
  expect(document.querySelector('[role="status"]')!.textContent).toContain("释放 10 B");
});

test("cleanup stale preview errors require rescan before another delete",async()=>{
  const actions={preview:async()=>({token:"stale",totalBytes:1,items:[{id:"one",path:"payload.part",bytes:1,category:"未完成下载"}]}),execute:async()=>{throw new Error("文件已变化，请重新扫描");}};
  await act(async()=>root.render(h(CleanupDialog,{actions,onClose:()=>{}})));
  await act(async()=>button("清理所选（1）").click());
  expect(document.querySelector('[role="alert"]')!.textContent).toContain("重新扫描");
  expect(button("清理所选（0）").disabled).toBe(true);
  await act(async()=>button("重新扫描").click());
  expect(button("清理所选（1）").disabled).toBe(false);
});

test("Burst download cards show TIFF plus shared XML while preserving legacy ZIP labels", async () => {
  const burst: DownloadTask = {...task("burst", "Completed"), product: "BURST",
    file: {name:"S1_burst.tiff",url:"https://example.invalid/burst.tiff",size:1024},
    companion: {name:"S1_source_VV.xml",url:"https://example.invalid/source.xml",size:20}};
  const legacy: DownloadTask = {...burst,id:"legacy",file:{...burst.file,name:"S1_old.zip"}};
  await act(async()=>root.render(h(DownloadCenter,{...centerProps,tasks:[burst,legacy]})));
  const cards=[...document.querySelectorAll(".download-item")];
  expect(cards.find(c=>c.textContent?.includes("S1_burst.tiff"))!.textContent).toContain("TIFF + XML");
  expect(cards.find(c=>c.textContent?.includes("S1_old.zip"))!.textContent).toContain("ZIP + XML");
  expect(document.querySelector('[title*="S1_source_VV.xml"]')).not.toBeNull();
});

test("relocated region actions retain coordinate mapping and remain disabled during a query", async () => {
  await act(async () => root.render(h(SarSearchForm, { areaSelector: h(RegionSelector), onReset: () => useSarSearchStore.getState().reset(), adding: false })));
  await act(async () => button("手动框选").click());
  expect(useMapStore.getState().drawing).toBe(true);
  const north = document.querySelector<HTMLInputElement>('[aria-label="北 (N)"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype,"value")!.set!.call(north,"25");
    north.dispatchEvent(new dom.window.Event("input",{bubbles:true}));
  });
  expect(useSarSearchStore.getState().request.bounds[3]).toBe(25);
  expect(useMapStore.getState().drawing).toBe(false);
  await act(async () => button("手动框选").click());
  await act(async () => useSarSearchStore.setState({busy:true}));
  expect(button("手动框选").matches(":disabled")).toBe(true);
  expect(button("边界文件").matches(":disabled")).toBe(true);
  await act(async () => button("手动框选").click());
  expect(useMapStore.getState().drawing).toBe(false);
});

test("dotted date input and calendar selection keep ISO values for queries", async () => {
  let value = "";
  function ControlledDate() {
    const [date, setDate] = useState("2024-01-02");
    value = date;
    return h(DatePicker, { label: "日期", value: date, onChange: setDate, displayFormat: "dots", calendarMax: "2024-12-31" });
  }
  await act(async () => root.render(h(ControlledDate)));
  const input = document.querySelector<HTMLInputElement>('[aria-label="日期"]')!;
  expect(input.value).toBe("2024.01.02");
  await act(async () => {
    Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value")!.set!.call(input, "2024.01.03");
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  });
  expect(value).toBe("2024-01-03");
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="选择日期"]')!.click());
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="2024-01-04"]')!.click());
  expect(value).toBe("2024-01-04");
  expect(input.value).toBe("2024.01.04");
});

test("date range still rejects an impossible dotted date", async () => {
  function ControlledDate() {
    const [date, setDate] = useState("2024-01-02");
    return h(DatePicker, { label: "日期", value: date, onChange: setDate, displayFormat: "dots", calendarMax: "2024-12-31" });
  }
  await act(async () => root.render(h(ControlledDate)));
  const input = document.querySelector<HTMLInputElement>('[aria-label="日期"]')!;
  await act(async () => {
    input.focus();
    Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value")!.set!.call(input, "2024.02.30");
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  });
  await act(async () => input.blur());
  expect(input.getAttribute("aria-invalid")).toBe("true");
  expect(document.querySelector('[role="alert"]')).not.toBeNull();
});

test("in-field product choices preserve NISAR/Sentinel compatibility", async () => {
  useSarSearchStore.getState().update({ productTypes: ["SLC"], satellites: ["Sentinel-1A"] });
  await act(async () => root.render(h(SarSearchForm, { areaSelector: null, onReset: () => {}, adding: false })));
  await act(async () => document.querySelector<HTMLButtonElement>('[role="combobox"][aria-label="产品类型"]')!.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  await act(async () => options.find((el) => el.textContent === "不限")!.click());
  await act(async () => options.find((el) => el.textContent?.includes("GSLC"))!.click());
  expect(useSarSearchStore.getState().request.productTypes).toEqual(["GSLC"]);
  expect(useSarSearchStore.getState().request.satellites).toEqual([]);
  expect(document.querySelector<HTMLButtonElement>('[aria-label="S1 卫星"]')!.disabled).toBe(true);
});

test("empty download center starts collapsed and Escape restores launcher focus", async () => {
  await act(async () => root.render(h(DownloadCenter, centerProps)));
  const launcher = document.querySelector<HTMLButtonElement>(".download-launcher")!;
  const drawer = document.querySelector(".download-drawer")!;
  expect(launcher.getAttribute("aria-expanded")).toBe("false");
  expect(drawer.hasAttribute("inert")).toBe(true);
  await act(async () => launcher.click());
  expect(drawer.hasAttribute("inert")).toBe(false);
  await act(async () => launcher.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(launcher.getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(launcher);
});

test("drawer preserves task filters across close/open and delegates real task ids", async () => {
  const actions: unknown[] = [];
  await act(async () => root.render(h(DownloadCenter, { ...centerProps, tasks: [task("active", "Downloading"), task("done", "Completed")], onAction: async (...args) => { actions.push(args); } })));
  const launcher = document.querySelector<HTMLButtonElement>(".download-launcher")!;
  await act(async () => launcher.click());
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="暂停 active.h5"]')!.click());
  expect(actions).toEqual([["pause", ["active"]]]);
  await act(async () => button("已完成").click());
  await act(async () => launcher.click());
  await act(async () => launcher.click());
  expect(button("已完成").getAttribute("aria-pressed")).toBe("true");
  expect(document.querySelectorAll('[role="listitem"]').length).toBe(1);
});

test("new submission reveals drawer but later progress cannot reopen a dismissed drawer", async () => {
  await act(async () => root.render(h(DownloadCenter, { ...centerProps, revealVersion: 0 })));
  await act(async () => root.render(h(DownloadCenter, { ...centerProps, tasks: [task("active", "Downloading")], revealVersion: 1 })));
  const launcher = document.querySelector<HTMLButtonElement>(".download-launcher")!;
  expect(launcher.getAttribute("aria-expanded")).toBe("true");
  await act(async () => launcher.click());
  await act(async () => root.render(h(DownloadCenter, { ...centerProps, tasks: [{ ...task("active", "Downloading"), revision: 2 }], revealVersion: 1 })));
  expect(launcher.getAttribute("aria-expanded")).toBe("false");
});

test("titlebar download popover closes outside without stealing focus and handles global Escape", async () => {
  await act(async () => root.render(h(DownloadCenter, centerProps)));
  const launcher = document.querySelector<HTMLButtonElement>(".download-launcher")!;
  expect(launcher.textContent).toBe("下载(0)");
  await act(async () => launcher.click());
  const outside = document.createElement("button");document.body.append(outside);outside.focus();
  await act(async () => outside.dispatchEvent(new dom.window.Event("pointerdown",{bubbles:true})));
  expect(launcher.getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(outside);
  await act(async () => launcher.click());
  await act(async () => document.dispatchEvent(new dom.window.KeyboardEvent("keydown",{key:"Escape",bubbles:true})));
  expect(launcher.getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(launcher);
});

test("road capsule toggles the reference layer without a legend or redundant controls", async () => {
  function ControlledLayers() {
    const [vectors,setVectors] = useState(false);
    return h(MapLayerControls,{vectors,onVectorsChange:setVectors});
  }
  await act(async () => root.render(h(ControlledLayers)));
  const toggle = button("地名与道路");
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
  expect(document.querySelector('[aria-label="影像图例"]')).toBeNull();
  await act(async () => toggle.click());expect(toggle.getAttribute("aria-pressed")).toBe("true");
  await act(async () => toggle.click());expect(toggle.getAttribute("aria-pressed")).toBe("false");
  expect(document.querySelector('[aria-label="重新加载底图"]')).toBeNull();
  expect(document.querySelector(".basemap-name")).toBeNull();
});

test("settings opened during startup show restored values and save credentials automatically", async () => {
  let saved: any; let closes = 0;
  const callbacks = { onSave: async (next: unknown) => { saved=next; }, onClose: () => { closes++; } };
  await act(async () => root.render(h(SettingsDialog,{settings:defaults,loading:true,...callbacks})));
  expect(document.querySelector<HTMLInputElement>('[aria-label="Earthdata 用户名"]')!.matches(":disabled")).toBe(true);
  const restored = {...defaults,outputDir:"E:/fixture",username:" fixture-user ",password:"synthetic-secret",parallelFiles:5};
  await act(async () => root.render(h(SettingsDialog,{settings:restored,loading:false,...callbacks})));
  expect(document.querySelector<HTMLInputElement>('[aria-label="Earthdata 用户名"]')!.value).toBe(" fixture-user ");
  await act(async () => document.querySelector("form")!.dispatchEvent(new dom.window.Event("submit",{bubbles:true,cancelable:true})));
  expect(saved.username).toBe("fixture-user");expect(saved.password).toBe("synthetic-secret");
  expect(saved.rememberPassword).toBe(true);expect(saved.parallelFiles).toBe(5);expect(closes).toBe(1);
});

test("settings password visibility and clear account controls preserve explicit removal on save", async () => {
  let saved: any;
  await act(async () => root.render(h(SettingsDialog,{settings:{...defaults,outputDir:"E:/fixture",username:"fixture-user",password:"synthetic-secret",rememberPassword:true},onSave:async next=>{saved=next;},onClose:()=>{}})));
  const password=document.querySelector<HTMLInputElement>('[aria-label="Earthdata 密码"]')!;
  expect(password.type).toBe("password");
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="显示密码"]')!.click());
  expect(password.type).toBe("text");
  await act(async () => button("清空账号").click());expect(password.value).toBe("");
  await act(async () => document.querySelector("form")!.dispatchEvent(new dom.window.Event("submit",{bubbles:true,cancelable:true})));
  expect(saved.username).toBe("");expect(saved.password).toBe("");expect(saved.rememberPassword).toBe(false);
});

test("settings save failure keeps the dialog editable and reports the actual error", async () => {
  let closes=0;
  await act(async () => root.render(h(SettingsDialog,{settings:{...defaults,outputDir:"E:/fixture"},onSave:async()=>{throw Error("synthetic settings write failure");},onClose:()=>{closes++;}})));
  await act(async () => document.querySelector("form")!.dispatchEvent(new dom.window.Event("submit",{bubbles:true,cancelable:true})));
  expect(document.querySelector('[role="alert"]')!.textContent).toContain("synthetic settings write failure");
  expect(document.querySelector<HTMLDialogElement>("dialog")!.open).toBe(true);expect(closes).toBe(0);
  expect(button("保存设置").disabled).toBe(false);
});

test("global interaction policy blocks context menus on portal content and inputs but permits input text selection", () => {
  const cleanup = installUiInteractionPolicy(document);
  const portal = document.createElement("div");
  portal.innerHTML = '<span>Result name</span><input type="text" value="editable"><textarea>notes</textarea>';
  document.body.append(portal);
  try {
    for (const target of [portal.querySelector("span")!, portal.querySelector("input")!]) {
      const event = new dom.window.MouseEvent("contextmenu", {bubbles:true,cancelable:true});
      target.dispatchEvent(event); expect(event.defaultPrevented).toBe(true);
    }
    const outside = new dom.window.Event("selectstart",{bubbles:true,cancelable:true});
    portal.querySelector("span")!.dispatchEvent(outside);expect(outside.defaultPrevented).toBe(true);
    for (const target of [portal.querySelector("input")!,portal.querySelector("textarea")!]) {
      const event = new dom.window.Event("selectstart",{bubbles:true,cancelable:true});
      target.dispatchEvent(event);expect(event.defaultPrevented).toBe(false);
    }
  } finally { cleanup();portal.remove(); }
  const event = new dom.window.MouseEvent("contextmenu",{bubbles:true,cancelable:true});
  document.body.dispatchEvent(event);expect(event.defaultPrevented).toBe(false);
});

test("result directory selection preserves partial checks and exported scene information", async () => {
  const scenes=["g1","g2","r1"].map((id)=>({id,granule:`granule-${id}`,name:`${id}.h5`,product:id==="r1"?"RSLC":"GSLC",url:`https://example.invalid/${id}`,size:1024,date:"2026-06-01T12:34:56Z",platform:"NISAR",polarization:"HH",direction:"ASCENDING",relativeOrbit:23,geometry:{type:"Polygon" as const,coordinates:[[[0,0],[1,0],[1,1],[0,0]]]}}));
  useSarSearchStore.setState({results:scenes});
  let exported: string[]=[]; let downloaded: string[]=[];
  function Results() { const model=useSarResults(true);return h(SarResultsPanel,{model,adding:false,onAdd:()=>{downloaded=model.products.map(item=>item.id);},onImport:()=>{exported=model.products.map(item=>item.id);}}); }
  await act(async()=>root.render(h(Results)));
  await act(async()=>button("全部展开").click());
  await act(async()=>document.querySelector<HTMLInputElement>('[aria-label="选择 NISAR · 升轨 · Path 23 中的 GSLC 影像"]')!.click());
  expect(useSarSearchStore.getState().selected).toEqual(["g1","g2"]);
  expect(document.querySelector<HTMLInputElement>('[aria-label="选择分组 NISAR · 升轨 · Path 23"]')!.indeterminate).toBe(true);
  await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="查看 granule-g1 详情"]')!.click());
  expect(document.querySelector(".sar-scene-details")!.textContent).toContain("g1.h5");
  expect(document.querySelector(".sar-scene-details")!.textContent).toContain("2026-06-01T12:34:56Z（UTC）");
  await act(async()=>button("导出所选 GeoJSON").click());expect(exported).toEqual(["g1","g2"]);
  await act(async()=>button("下载所选（2 景）").click());expect(downloaded).toEqual(["g1","g2"]);
  await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="折叠 NISAR · 升轨 · Path 23 中的 GSLC"]')!.click());
  expect(document.querySelector('[aria-label="定位影像 granule-g1"]')).toBeNull();
  expect(document.querySelector('[aria-label="定位影像 granule-r1"]')).not.toBeNull();
  await act(async()=>useSarSearchStore.getState().activateScene("g1",true));
  expect(document.querySelector('[aria-label="定位影像 granule-g1"]')).not.toBeNull();
  expect(useSarSearchStore.getState().selected).toEqual(["g1","g2"]);
  await act(async()=>button("全部折叠").click());
  expect(document.querySelectorAll(".sar-scene-item").length).toBe(0);
});

test("delete all includes hidden and active tasks and blocks repeat actions until completion", async () => {
  const actions: unknown[]=[];let finish!:()=>void;
  const tasks=[task("active","Downloading"),task("done","Completed"),task("failed","Failed")];
  await act(async()=>root.render(h(DownloadCenter,{...centerProps,tasks,onAction:async(...args)=>{actions.push(args);await new Promise<void>(resolve=>{finish=resolve;});}})));
  await act(async()=>document.querySelector<HTMLButtonElement>(".download-launcher")!.click());
  await act(async()=>button("已完成").click());
  await act(async()=>button("清空记录").click());
  expect(actions).toEqual([["remove",["active","done","failed"]]]);
  expect(button("清空中…").disabled).toBe(true);expect(button("全部开始").disabled).toBe(true);
  expect(document.querySelector('[role="status"]')!.textContent).toContain("本地文件保留");
  await act(async()=>finish());
  expect(button("清空记录").disabled).toBe(false);
  await act(async()=>root.render(h(DownloadCenter,{...centerProps,tasks:[]})));
  expect(button("清空记录").disabled).toBe(true);
});

test("transfer settings save 32 files and 64 connections and reject larger or fractional values", async () => {
  const saved: any[]=[];
  await act(async()=>root.render(h(SettingsDialog,{settings:{...defaults,outputDir:"E:/fixture"},onSave:async next=>{saved.push(next);},onClose:()=>{}})));
  const files=document.querySelector<HTMLInputElement>('[aria-label="同时下载数"]')!;
  const connections=document.querySelector<HTMLInputElement>('[aria-label="单文件连接数"]')!;
  const setValue=async(input:HTMLInputElement,value:string)=>act(async()=>{
    Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype,"value")!.set!.call(input,value);
    input.dispatchEvent(new dom.window.Event("input",{bubbles:true}));
  });
  const submit=async()=>act(async()=>document.querySelector("form")!.dispatchEvent(new dom.window.Event("submit",{bubbles:true,cancelable:true})));
  await setValue(files,"32");await setValue(connections,"64");
  expect(files.checkValidity()).toBe(true);expect(connections.checkValidity()).toBe(true);
  await submit();expect(saved[0].parallelFiles).toBe(32);expect(saved[0].concurrency).toBe(64);
  await setValue(files,"33");expect(files.checkValidity()).toBe(false);await submit();expect(saved.length).toBe(1);
  expect(document.querySelector('[role="alert"]')!.textContent).toContain("1–32");
  await setValue(files,"32");await setValue(connections,"65");expect(connections.checkValidity()).toBe(false);await submit();expect(saved.length).toBe(1);
  expect(document.querySelector('[role="alert"]')!.textContent).toContain("1–64");
  await setValue(connections,"1.5");expect(connections.checkValidity()).toBe(false);await submit();expect(saved.length).toBe(1);
});


test("collapsed downloads have no display timer; opening and closing preserves filters",async()=>{
  const originalSet=window.setInterval.bind(window),originalClear=window.clearInterval.bind(window);
  const active=new Set<number>();
  window.setInterval=((handler:any,ms?:number)=>{const id=originalSet(handler,ms);active.add(id);return id;}) as typeof window.setInterval;
  window.clearInterval=((id?:number)=>{active.delete(id!);originalClear(id);}) as typeof window.clearInterval;
  try {
    await act(async()=>root.render(h(DownloadCenter,{...centerProps,tasks:[task("active","Downloading")]})));
    expect(active.size).toBe(0);
    await act(async()=>document.querySelector<HTMLButtonElement>(".download-launcher")!.click());
    expect(active.size).toBe(1);
    await act(async()=>button("已完成").click());
    await act(async()=>document.querySelector<HTMLButtonElement>(".download-launcher")!.click());
    expect(active.size).toBe(0);
    await act(async()=>document.querySelector<HTMLButtonElement>(".download-launcher")!.click());
    expect(button("已完成").getAttribute("aria-pressed")).toBe("true");
    await act(async()=>root.render(null));expect(active.size).toBe(0);
  } finally {window.setInterval=originalSet;window.clearInterval=originalClear;}
});

test("GeoJSON export locks duplicate clicks and unlocks with error feedback",async()=>{
  const {SarDownloadView}=await import("../src/features/sar-download/components/SarDownloadView");
  const scene={id:"export",granule:"export",name:"export.h5",product:"GSLC",url:"https://example.invalid/export",size:1,date:"2026-06-01",platform:"NISAR",polarization:"HH",direction:"ASCENDING",relativeOrbit:23,geometry:{type:"Polygon" as const,coordinates:[[[0,0],[1,0],[1,1],[0,0]]]}};
  useSarSearchStore.setState({results:[scene],selected:[scene.id]});
  let calls=0;let reject!:(error:Error)=>void;
  await act(async()=>root.render(h(SarDownloadView,{areaSelector:null,onReset:()=>{},onDownload:async()=>{},onImport:async()=>{calls++;await new Promise<void>((_,no)=>{reject=no;});}})));
  await act(async()=>{const trigger=button("导出所选 GeoJSON");trigger.click();trigger.click();});
  expect(calls).toBe(1);expect(button("导出中…").disabled).toBe(true);
  await act(async()=>reject(new Error("disk unavailable")));
  expect(button("导出所选 GeoJSON").disabled).toBe(false);
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("disk unavailable");
});


test("help opens on first visit, persists seen flag, and can be reopened manually",async()=>{
  const {HelpDialog}=await import("../src/features/help/HelpDialog");
  const {useHelp,HELP_SEEN_KEY}=await import("../src/features/help/useHelp");
  dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
  localStorage.removeItem(HELP_SEEN_KEY);
  const source="# 操作指南\n\n## 开始使用\n\n1. 点击 **设置**。\n2. 选择 `SLC`。\n\n| 操作 | 说明 |\n|---|---|\n| 下载 | 保存影像 |";
  function HelpHarness(){const help=useHelp();return h("div",null,h("button",{onClick:help.show},"帮助"),help.open&&h(HelpDialog,{source,onClose:help.close}));}
  await act(async()=>root.render(h(HelpHarness)));
  expect(document.querySelector('dialog')?.open).toBe(true);
  expect(localStorage.getItem(HELP_SEEN_KEY)).toBe("1");
  expect(document.querySelector('strong')?.textContent).toBe("设置");
  expect(document.querySelector('code')?.textContent).toBe("SLC");
  expect(document.querySelectorAll('tbody tr').length).toBe(1);
  expect(document.querySelectorAll('ol li').length).toBe(2);
  expect(document.querySelector('[aria-label="帮助目录"]')?.textContent).toContain("开始使用");
  await act(async()=>document.querySelector<HTMLButtonElement>('.help-dialog footer button')!.click());expect(document.querySelectorAll('dialog').length).toBe(0);
  await act(async()=>root.render(null));
  await act(async()=>root.render(h(HelpHarness)));expect(document.querySelectorAll('dialog').length).toBe(0);
  await act(async()=>button("帮助").click());expect(document.querySelector('dialog')?.open).toBe(true);
  const cancel=new dom.window.Event("cancel",{cancelable:true});
  await act(async()=>document.querySelector('dialog')!.dispatchEvent(cancel));
  expect(cancel.defaultPrevented).toBe(true);expect(document.querySelectorAll('dialog').length).toBe(0);
});

test("help displays the current guide without evaluating raw HTML",async()=>{
  const {GuideContent,guideSections}=await import("../src/features/help/GuideContent");
  const {readFileSync}=await import("node:fs");
  const guide=readFileSync(new URL("../docs/新用户操作指南.md",import.meta.url),"utf8");
  await act(async()=>root.render(h(GuideContent,{source:guide+"\n\n<script>alert(1)</script>"})));
  expect(guideSections(guide).length).toBe(12);
  expect(document.querySelectorAll('h3').length).toBe(12);
  expect(document.querySelectorAll('table').length).toBe(7);
  expect(document.querySelectorAll('script').length).toBe(0);
  expect(document.body.textContent).toContain("<script>alert(1)</script>");
  expect(document.body.textContent).toContain("遇到问题时提供哪些信息");
});
