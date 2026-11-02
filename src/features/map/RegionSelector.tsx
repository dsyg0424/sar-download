import { RegionActionsContext } from "./RegionActionsContext";
import { boundaryAccept, importBoundary } from "./importBoundary";
import { chooseBoundaryFiles } from "./boundaryPicker";
import { useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { FileUp, MapPin, Square } from "lucide-react";
import { Select } from "../../shared/components/Select";
import { useSarSearchStore } from "../sar-download/store/useSarSearchStore";
import { boundsRegion, regionBounds } from "./region";
import { useMapStore } from "./useMapStore";
import "./area-selector.css";

type AdminRegion = { name: string; code: string; level: "province" | "city" | "county"; bounds: { west: number; south: number; east: number; north: number } };
const list = (parentCode: string | null, level: string) => invoke<AdminRegion[]>("list_admin_regions", { parentCode, level });
export function RegionSelector() {
  const store = useSarSearchStore();
  const actionsHost = useContext(RegionActionsContext);
  const { drawing, setDrawing, focusRegion } = useMapStore();
  const [error, setError] = useState("");
  const [importing, setImporting] = useState(false);
  const [filename, setFilename] = useState("");
  const [draft, setDraft] = useState(["", "", "", ""]);
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [regions, setRegions] = useState<AdminRegion[][]>([[], [], []]);
  const [selected, setSelected] = useState(["", "", ""]);
  const [position, setPosition] = useState({ left: 0, top: 0, width: 0 });
  const trigger = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const revision = useRef(0);
  const importRevision = useRef<number | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const localBounds = useRef<number[] | null>(null);
  useEffect(() => { if (store.request.bounds !== localBounds.current) setDraft(Array.from({ length: 4 }, (_, i) => Number.isFinite(store.request.bounds[i]) ? String(store.request.bounds[i]) : "")); }, [store.request.bounds]);
  useEffect(() => { setSelected(["", "", ""]); setFilename(""); setError(""); setOpen(false); setDrawing(false); setImporting(false); revision.current++; }, [store.resetVersion, setDrawing]);
  useEffect(() => { if (store.busy) { setOpen(false); setDrawing(false); } }, [store.busy, setDrawing]);
  useEffect(() => () => { revision.current++; }, []);
  useEffect(() => {
    if (!open) return;
    const place = () => { const r = trigger.current!.getBoundingClientRect(); const width = Math.min((trigger.current!.closest(".region-actions-host") ?? trigger.current!.closest("fieldset"))!.getBoundingClientRect().width, window.innerWidth - 10); setPosition({ left: Math.max(5, Math.min(r.left, window.innerWidth - width - 5)), top: Math.max(5, Math.min(r.bottom + 5, window.innerHeight - 130)), width }); };
    place();
    const outside = (event: PointerEvent) => {
      const target = event.target as HTMLElement;
      if (!popover.current?.contains(target) && !trigger.current?.contains(target) && !target.closest(".ui-select-menu")) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { setOpen(false); trigger.current?.focus(); } };
    document.addEventListener("pointerdown", outside); window.addEventListener("resize", place); window.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside); window.removeEventListener("resize", place); window.removeEventListener("keydown", escape); };
  }, [open]);
  const choose = async (column: number, code: string) => {
    const token = ++revision.current;
    setLoading(false);
    const next = selected.map((value, i) => i === column ? code : i > column ? "" : value);
    setSelected(next); setError("");
    setRegions((current) => current.map((rows, i) => i > column ? [] : rows));
    const region = regions[column].find((r) => r.code === code); if (!region) return;
    const b = region.bounds;
    store.update({ bounds: [b.west, b.south, b.east, b.north], geometry: null }); setFilename(""); focusRegion(); setDrawing(false);
    if (column === 2 || region.level === "county") return;
    setLoading(true);
    try {
      const children = await list(code, column === 0 ? "city" : "county");
      if (revision.current === token) setRegions((current) => current.map((rows, i) => i === column + 1 ? children : rows));
    } catch (e) { if (revision.current === token) setError(String(e)); }
    finally { if (revision.current === token) setLoading(false); }
  };
  const coordinate = (index: number, label: string) => <div className={`area-compass-cell ${["west", "south", "east", "north"][index]}`}><span className="area-coordinate-axis" aria-hidden="true">{["W", "S", "E", "N"][index]}</span><input
    className="area-coordinate-input" type="number" step="any" aria-label={label} placeholder={label} value={draft[index]}
    onChange={(event) => {
      revision.current++; setLoading(false); setDrawing(false); setFilename(""); setSelected(["", "", ""]); setError("");
      const next = [...draft]; next[index] = event.target.value; setDraft(next);
      const bounds = next.map((value) => value.trim() ? Number(value) : NaN);
      // Keep partially typed values local while invalidating stale search coordinates.
      localBounds.current = bounds; store.update({ bounds, geometry: null });
    }} /></div>;
  const directCounty = regions[1][0]?.level === "county";
  const loadBoundary = async (select: () => Promise<File[] | null>) => {
    const token = ++revision.current;
    importRevision.current = token;
    setImporting(true); setError("");
    try {
      const files = await select();
      if (!files?.length || token !== revision.current || useSarSearchStore.getState().busy) return;
      const geometry = await importBoundary(files);
      if (token !== revision.current || useSarSearchStore.getState().busy) return;
      store.update({ geometry, bounds: regionBounds(geometry) }); setSelected(["", "", ""]);
      useSarSearchStore.setState({ areaSelected: true });
      setFilename(files.find((file) => /\.shp$/i.test(file.name))?.name ?? files[0].name);
      focusRegion();
    } catch (e) { if (token === revision.current) setError(String(e)); }
    finally { if (importRevision.current === token) { importRevision.current = null; setImporting(false); } }
  };
  const actions = (<div className="area-submode-tabs">
      <button ref={trigger} type="button" className={`area-submode-btn ${open ? "active" : ""}`} aria-expanded={open} aria-controls={open ? "admin-regions" : undefined} onClick={async () => {
        setDrawing(false); setOpen(!open); if (open || regions[0].length) return;
        if (!isTauri()) { setError("行政区划选择请在桌面应用中使用。"); return; }
        const token = ++revision.current; setLoading(true); setError("");
        try { const provinces = await list(null, "province"); if (token === revision.current) setRegions([provinces, [], []]); }
        catch (e) { if (token === revision.current) setError(String(e)); }
        finally { if (token === revision.current) setLoading(false); }
      }} title="按行政区外包矩形设置检索范围"><MapPin size={13} />行政区划</button>
      <button type="button" className="area-submode-btn" disabled={importing} onClick={() => {
        setOpen(false); setDrawing(false);
        if (isTauri()) void loadBoundary(chooseBoundaryFiles);
        else file.current?.click();
      }} title="导入 GeoJSON、KML、KMZ 或 Shapefile；桌面端选择 SHP 自动读取同目录同名 PRJ"><FileUp size={13} />边界文件</button>
      <button type="button" className={`area-submode-btn ${drawing ? "active" : ""}`} aria-pressed={drawing} onClick={() => { setOpen(false); setDrawing(!drawing); }} title="在地图拖动框选，或点击两个对角点；Esc 取消"><Square size={13} />手动框选</button>
    </div>);
  return <fieldset className="area-card area-area-card" aria-label="检索范围" disabled={store.busy}>
    <div className="area-coordinate-grid">
      {coordinate(3, "北 (N)")}
      {coordinate(0, "西 (W)")}{coordinate(2, "东 (E)")}
      {coordinate(1, "南 (S)")}
    </div>
    {actionsHost ? createPortal(actions, actionsHost) : null}
    <input ref={file} type="file" hidden multiple accept={boundaryAccept} aria-label="导入检索边界" onChange={async (event) => {
      const input = event.currentTarget; const selectedFiles = Array.from(input.files ?? []); if (!selectedFiles.length) return;
      try { await loadBoundary(async () => selectedFiles); } finally { input.value = ""; }
    }} />
    {importing && <p className="area-inline-message" role="status">正在解析边界…</p>}
    {store.request.geometry && <p className="area-inline-message" title={filename}>{filename || "已导入精确边界"}<button type="button" onClick={() => { store.update({ geometry: null }); setFilename(""); }}>改用矩形</button></p>}
    {!boundsRegion(store.request.bounds) && draft.some(Boolean) && <p className="area-inline-message">请输入有效的经纬度，西小于东、南小于北。</p>}
    {error && <p role="alert" className="area-inline-message error-message">{error}</p>}
    {open && createPortal(<div id="admin-regions" role="group" aria-label="行政区划选择" ref={popover} className="area-admin-inline-popover area-admin-portal" style={position}>
      <div className={`area-admin-grid ${directCounty ? "two-cols" : ""}`}>
        {["省级", directCounty ? "县级" : "市级", "县级"].map((label, index) => directCounty && index === 2 ? null : <label key={index} className="area-admin-select-label"><span className="area-admin-label-title">{label}</span>
          <Select aria-label={label} disabled={loading || !regions[index].length || store.busy} value={selected[index]} onChange={(e) => void choose(index, e.target.value)}>
            <option value="">{loading ? "加载中…" : "请选择"}</option>{regions[index].map((r) => <option key={r.code} value={r.code}>{r.name}</option>)}
          </Select></label>)}
      </div>
    </div>, document.body)}
  </fieldset>;
}
