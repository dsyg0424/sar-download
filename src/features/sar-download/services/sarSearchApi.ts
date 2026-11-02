import type * as GeoJSON from "geojson";
import { Channel, invoke } from "@tauri-apps/api/core";

export interface SarSearchRequest {
  productTypes?: string[];
  satellites?: string[];
  imageType?: string;
  bounds: number[];
  geometry?: GeoJSON.Polygon | GeoJSON.MultiPolygon | null;
  start: string;
  end: string;
  product?: string;
  beamMode?: string;
  platform?: string;
  direction: string;
  polarization: string;
}
export interface SarProduct {
  id: string;
  granule: string;
  name: string;
  url: string;
  size: number;
  date: string;
  platform: string;
  product: string;
  polarization: string;
  direction: string;
  relativeOrbit: number | null;
  geometry: GeoJSON.Polygon | GeoJSON.MultiPolygon;
  downloadHash?: { type: string; value: string } | null;
}
export interface SarSearchResponse {
  results: SarProduct[];
  limitReached: boolean;
  warnings?: string[];
}
export type SarSearchEvent =
  | { event: "product_started"; product: string }
  | {
      event: "stage";
      product: string;
      stage: string;
      state: "started" | "finished";
      elapsedMs?: number;
    }
  | { event: "product_complete" | "product_page"; product: string; data: SarSearchResponse };

export const searchSar = (
  request: SarSearchRequest,
  onProgress?: (event: SarSearchEvent) => void,
  requestId = crypto.randomUUID(),
) => {
  const progress = new Channel<SarSearchEvent>();
  progress.onmessage = (event) => onProgress?.(event);
  return invoke<SarSearchResponse>("sar_search", {
    progress,
    requestId,
    request:
      request.productTypes !== undefined
        ? {
            productTypes: request.productTypes,
            satellites: request.satellites ?? [],
            bounds: request.bounds,
            geometry: request.geometry,
            start: request.start,
            end: request.end,
            direction: request.direction,
            polarization: request.polarization,
          }
        : request,
  });
};

export const cancelSar = (requestId: string) => invoke<void>("cancel_sar_search", { requestId });
