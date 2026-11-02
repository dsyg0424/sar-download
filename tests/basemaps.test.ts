import { expect, test } from "bun:test";
import { basemapState, isBasemapLoaded, rasterSource, type BasemapState } from "../src/features/map/basemaps";
import { validateStyleMin } from "@maplibre/maplibre-gl-style-spec";
import { REFERENCE_SOURCE, referenceLayers, referenceResources } from "../src/features/map/reference";

test("ArcGIS reference overlay is a valid transparent vector style with fonts and icons", () => {
  const resources = referenceResources(true);
  expect(resources.source.tiles).toEqual(["sar-vector://tiles/{z}/{y}/{x}.pbf"]);
  expect(resources.glyphs).toContain("{fontstack}/{range}.pbf");
  expect(resources.sprite).toBe("sar-vector://sprites/sprite");
  expect(referenceLayers.some((layer) => layer.type === "line")).toBe(true);
  expect(referenceLayers.some((layer) => layer.type === "symbol")).toBe(true);
  expect(referenceLayers.every((layer) => layer.type === "line" || layer.type === "symbol")).toBe(true);
  expect(validateStyleMin({ version: 8, glyphs: resources.glyphs, sprite: resources.sprite,
    sources: { [REFERENCE_SOURCE]: resources.source }, layers: referenceLayers })).toEqual([]);
});

test("only decoded raster tiles mark the base loaded, including when the viewport has no tiles", () => {
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: true, tile: { state: "loaded" } }, "base")).toBe(true);
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: true, sourceDataType: "idle" }, "base")).toBe(false);
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: true, sourceDataType: "content" }, "base")).toBe(false);
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: true, tile: { state: "errored" } }, "base")).toBe(false);
  expect(isBasemapLoaded({ sourceId: "footprints", isSourceLoaded: true, tile: {} }, "base")).toBe(false);
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: false, tile: {} }, "base")).toBe(false);
  expect(isBasemapLoaded({ sourceId: "base", isSourceLoaded: true, sourceDataType: "metadata" }, "base")).toBe(false);
});

test("unrelated overlay events and late events cannot clear a basemap failure", () => {
  let state: BasemapState = { phase: "loading", source: "satellite-1", error: "" };
  state = basemapState(state, { type: "error", source: "satellite-1", error: "offline" });
  expect(state.phase).toBe("error");
  expect(basemapState(state, { type: "loaded", source: "products" })).toEqual(state);
  expect(basemapState(state, { type: "loaded", source: "satellite-1" })).toEqual(state);
  const retry = basemapState(state, { type: "start", source: "satellite-2" });
  expect(retry.error).toBe("");
  expect(basemapState(retry, { type: "error", source: "satellite-1" })).toEqual(retry);
  expect(basemapState(retry, { type: "loaded", source: "satellite-2" }).phase).toBe("ready");
});
test("native tiles use the Rust transport and satellite zooms reuse existing high-resolution tiles", () => {
  const imagery = rasterSource(3, true);
  expect(imagery.tiles![0]).toBe("sar-tile://imagery/{z}/{x}/{y}?retry=3");
  expect(imagery.maxzoom).toBe(17);
  expect(rasterSource(0, false).tiles).toEqual(["https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"]);
  expect(rasterSource(0, false).attribution).toContain("Esri");
});
