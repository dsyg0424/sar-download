import type { LayerSpecification, VectorSourceSpecification } from "maplibre-gl";
import { isTauri } from "@tauri-apps/api/core";
import hybridStyle from "./arcgis-hybrid.json?raw";

// Esri Hybrid Reference Layer, item 30d6b8271e1849cd9c3042060001f425.
// This style contains transparent line/symbol layers, with no opaque basemap.
export const REFERENCE_SOURCE = "arcgis-reference";
const service = "https://basemaps.arcgis.com/arcgis/rest/services/World_Basemap_v2/VectorTileServer";
export function referenceResources(native = isTauri()) {
  return {
    source: { type: "vector", tiles: [native ? "sar-vector://tiles/{z}/{y}/{x}.pbf" : `${service}/tile/{z}/{y}/{x}.pbf`], minzoom: 0, maxzoom: 22,
      attribution: "Reference © Esri, TomTom, Garmin, FAO, NOAA, USGS, © OpenStreetMap contributors" } satisfies VectorSourceSpecification,
    glyphs: native ? "sar-vector://fonts/{fontstack}/{range}.pbf" : `${service}/resources/fonts/{fontstack}/{range}.pbf`,
    sprite: native ? "sar-vector://sprites/sprite" : "https://cdn.arcgis.com/sharing/rest/content/items/30d6b8271e1849cd9c3042060001f425/resources/sprites/sprite",
  };
}
export const referenceLayers = (JSON.parse(hybridStyle).layers as LayerSpecification[]).map((layer) => ({
  ...layer, id: `arcgis-reference/${layer.id}`, source: REFERENCE_SOURCE,
})) as LayerSpecification[];
