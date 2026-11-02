import { bindScenePicking } from "./scenePicking";
import { MapLayerControls } from "./MapLayerControls";
import type * as GeoJSON from "geojson";
import { useEffect, useRef, useState } from "react";
import { Map, setWorkerUrl, setMaxParallelImageRequests, type GeoJSONSource, type LngLat, type Point } from "maplibre-gl";
import mapWorkerUrl from "maplibre-gl/dist/maplibre-gl-csp-worker.js?url";
import { Loader2 } from "lucide-react";
import { useSarSearchStore } from "../sar-download/store/useSarSearchStore";
import { boundsRegion, regionBounds } from "./region";
import { useMapStore } from "./useMapStore";
import { basemapState, isBasemapLoaded, rasterSource, registerTileProtocol, type BasemapState } from "./basemaps";
import { REFERENCE_SOURCE, referenceLayers, referenceResources } from "./reference";

const empty: GeoJSON.FeatureCollection = { type: "FeatureCollection", features: [] };
function viewport(): { center: [number, number]; zoom: number } {
  try {
    const saved = JSON.parse(localStorage.getItem("sar-download.viewport") || "null");
    if (Array.isArray(saved?.center) && saved.center.length === 2 && saved.center.every(Number.isFinite) && Math.abs(saved.center[0]) <= 180 && Math.abs(saved.center[1]) <= 85 && Number.isFinite(saved.zoom) && saved.zoom >= 0 && saved.zoom <= 20) return saved;
  } catch { /* Use the default view when storage is unavailable. */ }
  return { center: [105, 32], zoom: 3 };
}
export function SearchMap() {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<Map | null>(null);
  const [ready, setReady] = useState(false);
  const [mountGeneration, setMountGeneration] = useState(0);
  const [generation, setGeneration] = useState(0);
  const [vectors, setVectors] = useState(() => {
    try { return localStorage.getItem("sar-download.vectors") !== "false"; } catch { return true; }
  });
  const [fatalError, setFatalError] = useState("");
  const [referenceGeneration, setReferenceGeneration] = useState(0);
  const [referenceStatus, setReferenceStatus] = useState<BasemapState>({ phase: "loading", source: REFERENCE_SOURCE, error: "" });
  const [status, setStatus] = useState<BasemapState>({ phase: "loading", source: "", error: "" });
  const store = useSarSearchStore();
  const { drawing, focus } = useMapStore();
  useEffect(() => {
    if (!container.current) return;
    // Same-origin packaged worker: no blob: worker or unsafe-eval CSP exception.
    setWorkerUrl(mapWorkerUrl);
    registerTileProtocol(); setMaxParallelImageRequests(8); setFatalError("");
    let native: Map;
    try {
      native = new Map({ container: container.current, ...viewport(), maxZoom: 20,
        dragRotate: false, pitchWithRotate: false, touchPitch: false, attributionControl: false,
        // Initialize overlays independently of tile availability so failed networks
        // never block region selection or SAR footprints.
        style: { version: 8, glyphs: referenceResources().glyphs, sources: {}, layers: [{ id: "background", type: "background", paint: { "background-color": "#ffffff" } }] },
      });
    } catch (e) { setFatalError(`地图初始化失败：${String(e)}`); return; }
    map.current = native;
    native.on("load", () => {
      native.addSource("products", { type: "geojson", data: empty }); native.addSource("region", { type: "geojson", data: empty });
      native.addLayer({ id: "footprints", type: "fill", source: "products", paint: {
        "fill-color": ["case", ["get", "active"], "#ff5722", ["get", "selected"], "#00e5ff", "#1b9b88"],
        "fill-opacity": ["case", ["get", "active"], 0.35, ["get", "selected"], 0.22, 0.08],
      } });
      native.addLayer({ id: "outlines", type: "line", source: "products", paint: { "line-color": ["case", ["get", "active"], "#ff5722", ["get", "selected"], "#00e5ff", "#9bc4bc"], "line-width": ["case", ["get", "active"], 3, 1] } });
      native.addLayer({ id: "region-fill", type: "fill", source: "region", paint: { "fill-color": "#00e5ff", "fill-opacity": 0.1 } });
      native.addLayer({ id: "region-outline", type: "line", source: "region", paint: { "line-color": "#00e5ff", "line-width": 2, "line-dasharray": [3, 2] } });
      setReady(true);
    });
    const restore = () => {
      const request = useSarSearchStore.getState().request; const geometry = request.geometry || boundsRegion(request.bounds);
      (native.getSource("region") as GeoJSONSource | undefined)?.setData(geometry ? { type: "Feature", geometry, properties: {} } : empty);
    };
    let anchor: LngLat | null = null;
    let press: { point: Point; location: LngLat } | null = null;
    let ignoreClickUntil = 0;
    const coordinates = (a: LngLat, b: LngLat) => [Math.min(a.lng, b.lng), Math.min(a.lat, b.lat), Math.max(a.lng, b.lng), Math.max(a.lat, b.lat)];
    const valid = (bounds: number[]) => !!boundsRegion(bounds) && bounds[0] >= -180 && bounds[2] <= 180 && bounds[1] >= -90 && bounds[3] <= 90;
    const confirm = (a: LngLat, b: LngLat) => {
      const bounds = coordinates(a, b); if (!valid(bounds)) return;
      anchor = null; press = null; ignoreClickUntil = performance.now() + 120;
      useSarSearchStore.getState().update({ bounds, geometry: null }); useMapStore.getState().setDrawing(false);
    };
    const syncDrawing = () => {
      const enabled = useMapStore.getState().drawing;
      native.getCanvas().style.cursor = enabled ? "crosshair" : "";
      native.dragPan[enabled ? "disable" : "enable"](); native.boxZoom[enabled ? "disable" : "enable"](); native.doubleClickZoom[enabled ? "disable" : "enable"]();
      if (!enabled) { anchor = null; press = null; restore(); }
    };
    const unsubscribe = useMapStore.subscribe((next, previous) => { if (next.drawing !== previous.drawing) syncDrawing(); }); syncDrawing();
    const escape = (e: KeyboardEvent) => { if (e.key === "Escape") useMapStore.getState().setDrawing(false); };
    const outsideUp = (e: MouseEvent) => { if (press && !native.getCanvasContainer().contains(e.target as Node)) useMapStore.getState().setDrawing(false); };
    window.addEventListener("keydown", escape); window.addEventListener("mouseup", outsideUp);
    native.on("mousedown", (e) => {
      if (!useMapStore.getState().drawing || useSarSearchStore.getState().busy || e.originalEvent.button !== 0) return;
      e.preventDefault(); press = { point: e.point, location: e.lngLat };
    });
    native.on("mousemove", (e) => {
      if (!useMapStore.getState().drawing) return;
      const start = anchor || (press && e.point.dist(press.point) > 4 ? press.location : null); if (!start) return;
      const bounds = coordinates(start, e.lngLat); const geometry = boundsRegion(bounds);
      if (geometry && valid(bounds)) (native.getSource("region") as GeoJSONSource | undefined)?.setData({ type: "Feature", properties: {}, geometry });
    });
    native.on("mouseup", (e) => {
      const current = press; press = null;
      if (current && e.point.dist(current.point) > 4) confirm(current.location, e.lngLat);
    });
    native.on("click", (e) => {
      if (performance.now() < ignoreClickUntil || !useMapStore.getState().drawing) return;
      if (anchor) confirm(anchor, e.lngLat); else anchor = e.lngLat;
    });
    const stopPicking = bindScenePicking(native, () => performance.now() >= ignoreClickUntil && !useMapStore.getState().drawing);
    native.on("moveend", () => { try { const center = native.getCenter().wrap(); localStorage.setItem("sar-download.viewport", JSON.stringify({ center: [center.lng, center.lat], zoom: native.getZoom() })); } catch { /* Optional preference. */ } });
    native.on("webglcontextlost", () => setFatalError("图形加速上下文已丢失，请重新加载地图。"));
    const resize = new ResizeObserver(() => native.resize()); resize.observe(container.current);
    return () => { stopPicking(); unsubscribe(); window.removeEventListener("keydown", escape); window.removeEventListener("mouseup", outsideUp); resize.disconnect(); native.remove(); map.current = null; setReady(false); };
  }, [mountGeneration]);
  useEffect(() => {
    if (!ready || !map.current) return;
    const native = map.current; const source = `basemap-imagery-${generation}`;
    let paused = false; let disposed = false;
    setStatus((old) => basemapState(old, { type: "start", source }));
    native.addSource(source, rasterSource(generation));
    native.addLayer({ id: source, type: "raster", source, paint: { "raster-fade-duration": 150 } }, vectors && native.getLayer(referenceLayers[0].id) ? referenceLayers[0].id : "footprints");
    const pause = (error: string) => {
      if (disposed || paused) return; paused = true;
      if (native.getLayer(source)) native.setLayoutProperty(source, "visibility", "none");
      setStatus((old) => basemapState(old, { type: "error", source, error }));
    };
    const timeout = window.setTimeout(() => pause("卫星影像加载超时，请重试。"), 45000);
    const onError = (event: { sourceId?: string; error?: Error }) => {
      if (event.sourceId === source) pause(event.error?.message || "底图网络不可用");
    };
    const onData = (event: { sourceId?: string; isSourceLoaded?: boolean; tile?: { state?: string } }) => {
      if (isBasemapLoaded(event, source) && !paused) {
        window.clearTimeout(timeout); setStatus((old) => basemapState(old, { type: "loaded", source }));
      }
    };
    const onLoading = (event: { sourceId?: string }) => { if (event.sourceId === source && !paused) setStatus((old) => basemapState(old, { type: "start", source })); };
    native.on("error", onError); native.on("sourcedata", onData); native.on("sourcedataloading", onLoading);
    return () => {
      disposed = true; window.clearTimeout(timeout); native.off("error", onError); native.off("sourcedata", onData); native.off("sourcedataloading", onLoading);
      if (map.current !== native) return;
      if (native.getLayer(source)) native.removeLayer(source);
      if (native.getSource(source)) native.removeSource(source);
    };
  }, [ready, generation]);
  useEffect(() => {
    try { localStorage.setItem("sar-download.vectors", String(vectors)); } catch { /* Optional preference. */ }
    if (!ready || !map.current || !vectors) return;
    const native = map.current;
    const resources = referenceResources();
    let disposed = false;
    let failed = false;
    const update = (phase: BasemapState["phase"], error = "") => {
      if (!disposed) setReferenceStatus({ phase, source: REFERENCE_SOURCE, error });
    };
    update("loading");
    const timeout = window.setTimeout(() => { failed = true; update("error", "矢量参考图层加载超时，请重试。"); }, 60000);
    const onError = (event: { sourceId?: string; error?: Error }) => {
      if (event.sourceId === REFERENCE_SOURCE || event.error?.message.includes("矢量资源加载失败")) {
        failed = true; window.clearTimeout(timeout); update("error", event.error?.message || "矢量参考图层加载失败");
      }
    };
    const onData = (event: { sourceId?: string; isSourceLoaded?: boolean; tile?: { state?: string } }) => {
      if (!failed && isBasemapLoaded(event, REFERENCE_SOURCE)) { window.clearTimeout(timeout); update("ready"); }
    };
    native.on("error", onError); native.on("sourcedata", onData);
    try {
      native.setSprite(resources.sprite);
      native.addSource(REFERENCE_SOURCE, resources.source);
      for (const layer of referenceLayers) native.addLayer(layer, "footprints");
    } catch (error) {
      failed = true; window.clearTimeout(timeout); update("error", String(error));
    }
    return () => {
      disposed = true; window.clearTimeout(timeout); native.off("error", onError); native.off("sourcedata", onData);
      if (map.current !== native) return;
      for (const layer of [...referenceLayers].reverse()) if (native.getLayer(layer.id)) native.removeLayer(layer.id);
      if (native.getSource(REFERENCE_SOURCE)) native.removeSource(REFERENCE_SOURCE);
    };
  }, [ready, vectors, referenceGeneration]);
  useEffect(() => {
    if (!ready || !map.current) return;
    const selected = new Set(store.selected);
    (map.current.getSource("products") as GeoJSONSource).setData({ type: "FeatureCollection", features: store.results.map((product) => ({ type: "Feature", geometry: product.geometry, properties: { id: product.id, selected: selected.has(product.id), active: store.activeScene === product.id } })) });
    const geometry = store.request.geometry || boundsRegion(store.request.bounds);
    (map.current.getSource("region") as GeoJSONSource).setData(geometry ? { type: "Feature", geometry, properties: {} } : empty);
  }, [ready, store.results, store.selected, store.activeScene, store.request.bounds, store.request.geometry]);
  const fit = (bounds: number[]) => {
    if (!boundsRegion(bounds) || !container.current) return;
    const canvas = container.current.getBoundingClientRect();
    const dock = document.querySelector(".search-dock")?.getBoundingClientRect();
    const results = document.querySelector(".sar-results-panel")?.getBoundingClientRect();
    const top = Math.min(dock ? dock.bottom - canvas.top + 24 : 80, canvas.height * .55);
    const left = Math.min(results ? results.right - canvas.left + 24 : 24, canvas.width * .45);
    map.current?.fitBounds([[bounds[0], bounds[1]], [bounds[2], bounds[3]]], { padding: { top, right: 32, bottom: 32, left }, maxZoom: 15, duration: 450 });
  };
  useEffect(() => { if (ready && focus) fit(useSarSearchStore.getState().request.bounds); }, [ready, focus]);
  useEffect(() => { const product = store.results.find((p) => p.id === store.focusRequest?.scene); if (ready && product) fit(regionBounds(product.geometry)); }, [ready, store.focusRequest, store.results]);
  const retry = () => { if (fatalError) { setReady(false); setMountGeneration((v) => v + 1); } else setGeneration((v) => v + 1); };
  return <section className="search-map" aria-label="SAR 影像地图">
    <div className="map-canvas" ref={container} />
    <MapLayerControls vectors={vectors} onVectorsChange={setVectors} />
    {drawing && <div className="map-instruction" role="status">拖动或点击两个对角点 · Esc 取消</div>}
    {(fatalError || status.phase === "error") && <div className="map-warning" role="alert"><span>{fatalError || "底图加载失败，框选仍可使用。"}</span><button aria-label="重试底图加载" title={fatalError || status.error} onClick={retry}>重试</button></div>}
    {vectors && !fatalError && referenceStatus.phase !== "ready" && <div className={`map-reference-status ${referenceStatus.phase}`} role="status">
      {referenceStatus.phase === "loading" ? <><Loader2 size={12} className="spinner" />地名与道路加载中</> : <><span title={referenceStatus.error}>地名与道路加载失败</span><button aria-label="重试地名与道路加载" onClick={() => setReferenceGeneration((v) => v + 1)}>重试</button></>}
    </div>}
  </section>;
}
