import type * as GeoJSON from "geojson";
import type { SarProduct } from "../services/sarSearchApi";

export const pathKey = (item: SarProduct) =>
  `${item.platform === "NISAR" ? "NISAR:" : ""}${item.direction}:${item.relativeOrbit ?? "unknown"}`;
export function groupProducts(results: SarProduct[]) {
  const groups = new Map<
    string,
    { id: string; label: string; products: SarProduct[] }
  >();
  for (const product of results) {
    const id = pathKey(product);
    if (!groups.has(id))
      groups.set(id, {
        id,
        label: `${product.platform === "NISAR" ? "NISAR" : "Sentinel-1"} · ${product.direction === "ASCENDING" ? "升轨" : product.direction === "DESCENDING" ? "降轨" : "未知方向"} · Path ${product.relativeOrbit ?? "未知"}`,
        products: [],
      });
    groups.get(id)!.products.push(product);
  }
  return [...groups.values()]
    .sort((a, b) => a.id.localeCompare(b.id, undefined, { numeric: true }))
    .map((group) => ({
      ...group,
      products: group.products.sort((a, b) => a.date.localeCompare(b.date)),
    }));
}
export function footprints(
  results: SarProduct[],
  active: string | null,
): GeoJSON.FeatureCollection {
  const labels = new Map(groupProducts(results).map(group => [group.id, group.label]));
  return {
    type: "FeatureCollection",
    features: results.map((item) => ({
      type: "Feature",
      id: item.id,
      geometry: item.geometry,
      properties: {
        scene: item.id,
        product: item.product,
        groupLabel: labels.get(pathKey(item)),
        group: pathKey(item),
        selected: pathKey(item) === active,
      },
    })),
  };
}
export function featureBounds(
  data: GeoJSON.FeatureCollection,
): [[number, number], [number, number]] | null {
  const points: number[][] = [];
  const walk = (value: unknown): void => {
    if (!Array.isArray(value)) return;
    if (typeof value[0] === "number" && typeof value[1] === "number") {
      if (Number.isFinite(value[0]) && Number.isFinite(value[1]))
        points.push(value);
    } else value.forEach(walk);
  };
  data.features.forEach((feature) => {
    if (feature.geometry && "coordinates" in feature.geometry)
      walk(feature.geometry.coordinates);
  });
  if (!points.length) return null;
  return points.reduce<[[number, number], [number, number]]>(
    (b, p) => [
      [Math.min(b[0][0], p[0]), Math.min(b[0][1], p[1])],
      [Math.max(b[1][0], p[0]), Math.max(b[1][1], p[1])],
    ],
    [
      [Infinity, Infinity],
      [-Infinity, -Infinity],
    ],
  );
}

export function sarFocusBounds(results: SarProduct[], target: { group: string; scene?: string }) {
  return featureBounds(footprints(results.filter(item => target.scene
    ? item.id === target.scene
    : pathKey(item) === target.group), null));
}
