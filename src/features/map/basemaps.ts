import { invoke, isTauri } from "@tauri-apps/api/core";
import { addProtocol, type RasterSourceSpecification } from "maplibre-gl";

export const IMAGERY_TILE_URL = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
let registered = false;
let requestSequence = 0;
export function registerTileProtocol() {
  if (registered || !isTauri()) return;
  addProtocol("sar-tile", async (request, controller) => {
    const url = new URL(request.url);
    const [z, x, y] = url.pathname.split("/").filter(Boolean).map(Number);
    if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
    const requestId = `${Date.now()}-${++requestSequence}`;
    const cancel = () => { void invoke("cancel_map_tile", { requestId }).catch(() => {}); };
    controller.signal.addEventListener("abort", cancel, { once: true });
    try {
      const data = await invoke<ArrayBuffer>("load_map_tile", { requestId, provider: url.hostname, z, x, y });
      if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
      return { data };
    } catch (error) {
      if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
      throw error;
    } finally { controller.signal.removeEventListener("abort", cancel); }
  });
  addProtocol("sar-vector", async (request, controller) => {
    if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
    const requestId = `vector-${Date.now()}-${++requestSequence}`;
    const cancel = () => { void invoke("cancel_map_tile", { requestId }).catch(() => {}); };
    controller.signal.addEventListener("abort", cancel, { once: true });
    try {
      const data = await invoke<ArrayBuffer>("load_map_resource", { requestId, resource: request.url });
      if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
      return { data: request.type === "json" ? JSON.parse(new TextDecoder().decode(data)) : data };
    } catch (error) {
      if (controller.signal.aborted) throw new DOMException("Aborted", "AbortError");
      throw new Error(`矢量资源加载失败：${String(error)}`);
    } finally { controller.signal.removeEventListener("abort", cancel); }
  });
  registered = true;
}
export function rasterSource(generation = 0, native = isTauri()): RasterSourceSpecification {
  const url = native ? `sar-tile://imagery/{z}/{x}/{y}?retry=${generation}` : IMAGERY_TILE_URL;
  return { type: "raster", tiles: [url], tileSize: 256, minzoom: 0, maxzoom: 17,
    attribution: "Tiles © Esri — Source: Esri, Maxar, Earthstar Geographics, and the GIS User Community" };
}
export interface BasemapState { phase: "loading" | "ready" | "error"; source: string; error: string }
export function isBasemapLoaded(event: { sourceId?: string; isSourceLoaded?: boolean; sourceDataType?: string; tile?: { state?: string } }, source: string) {
  // Metadata/content/idle can fire with no visible tiles (including a zero-height
  // canvas). Only a decoded raster tile proves that imagery has actually loaded.
  return event.sourceId === source && event.isSourceLoaded === true && event.tile?.state === "loaded";
}
export function basemapState(state: BasemapState, event: { type: "start" | "loaded" | "error"; source: string; error?: string }): BasemapState {
  if (event.type === "start") return { phase: "loading", source: event.source, error: "" };
  if (event.source !== state.source || state.phase === "error") return state;
  return event.type === "loaded" ? { ...state, phase: "ready", error: "" } : { ...state, phase: "error", error: event.error || "底图加载失败" };
}
