import type * as GeoJSON from "geojson";
export type Region = GeoJSON.Polygon | GeoJSON.MultiPolygon;
export function parseRegion(text: string): Region {
  const value = JSON.parse(text.replace(/^\uFEFF/, "")) as GeoJSON.GeoJSON & { crs?: { properties?: { name?: string } } };
  const crs = value?.crs?.properties?.name;
  if (crs && !/^(urn:ogc:def:crs:OGC:1\.3:CRS84|urn:ogc:def:crs:EPSG::4326|EPSG:4326|CRS84)$/i.test(crs)) throw new Error("GeoJSON 需先转换为 WGS84 经纬度坐标");
  const polygons: GeoJSON.Position[][][] = [];
  const collect = (item: GeoJSON.GeoJSON | null, depth = 0) => {
    if (!item || depth > 16) throw new Error("边界为空或嵌套层级过多");
    if (item.type === "FeatureCollection") item.features.forEach((feature) => collect(feature, depth + 1));
    else if (item.type === "Feature") collect(item.geometry, depth + 1);
    else if (item.type === "GeometryCollection") item.geometries.forEach((geometry) => collect(geometry, depth + 1));
    else if (item.type === "Polygon") polygons.push(item.coordinates);
    else if (item.type === "MultiPolygon") polygons.push(...item.coordinates);
    else throw new Error("边界文件只能包含 Polygon / MultiPolygon 面，不能包含点或线");
  };
  collect(value);
  if (!polygons.length || polygons.some((polygon) => !Array.isArray(polygon) || !polygon.length || polygon.some((ring) => !Array.isArray(ring) || ring.length < 4 || ring[0]?.[0] !== ring[ring.length - 1]?.[0] || ring[0]?.[1] !== ring[ring.length - 1]?.[1]))) throw new Error("边界必须包含有效的闭合多边形环，每环至少 4 个坐标点");
  const region: Region = polygons.length === 1 ? { type: "Polygon", coordinates: polygons[0] } : { type: "MultiPolygon", coordinates: polygons };
  regionBounds(region); return region;
}
export function regionBounds(region: Region): [number, number, number, number] {
  const points = region.type === "Polygon" ? region.coordinates.flat() : region.coordinates.flat(2);
  if (!points.length || points.some((p) => !Array.isArray(p) || p.length < 2 || !p.slice(0, 2).every((v) => typeof v === "number" && Number.isFinite(v)) || Math.abs(p[0]) > 180 || Math.abs(p[1]) > 90)) throw new Error("边界需使用有效的 WGS84 经纬度坐标");
  const bounds: [number, number, number, number] = [180, 90, -180, -90];
  for (const p of points) { bounds[0] = Math.min(bounds[0], p[0]); bounds[1] = Math.min(bounds[1], p[1]); bounds[2] = Math.max(bounds[2], p[0]); bounds[3] = Math.max(bounds[3], p[1]); }
  if (bounds[0] >= bounds[2] || bounds[1] >= bounds[3]) throw new Error("GeoJSON 范围为空");
  return bounds;
}
export function boundsRegion(bounds: number[]): Region | null {
  const [w, s, e, n] = bounds;
  if (bounds.length !== 4 || !bounds.every(Number.isFinite) || w >= e || s >= n) return null;
  return { type: "Polygon", coordinates: [[[w, s], [e, s], [e, n], [w, n], [w, s]]] };
}
