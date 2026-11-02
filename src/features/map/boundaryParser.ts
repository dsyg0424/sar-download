import { DOMParser } from "@xmldom/xmldom";
import { kml } from "@tmcw/togeojson";
import { unzipSync } from "fflate";
import { parseShp } from "shpjs";
import proj4 from "proj4";
import { parseRegion, type Region } from "./region";

const inputLimit = 20 * 1024 * 1024;
const expandedLimit = 64 * 1024 * 1024;
const extension = (name: string) => name.slice(name.lastIndexOf(".")).toLowerCase();
const stem = (name: string) => name.slice(0, name.lastIndexOf(".")).toLowerCase();
const decode = (bytes: Uint8Array) => new TextDecoder("utf-8", { fatal: true }).decode(bytes).replace(/^\uFEFF/, "");

function parseKml(text: string): Region {
  if (/<!DOCTYPE|<!ENTITY/i.test(text)) throw new Error("KML 不支持 DTD 或实体声明");
  const doc = new DOMParser({ onError: (level, message) => { if (level !== "warning") throw new Error(message); } }).parseFromString(text, "text/xml");
  if (doc.documentElement?.localName !== "kml") throw new Error("文件不是有效的 KML 文档");
  return parseRegion(JSON.stringify(kml(doc)));
}

function shapeRegion(shp: Uint8Array, prj: Uint8Array | undefined): Region {
  if (!prj) throw new Error("Shapefile 缺少同名 PRJ；请同时选择 SHP 与 PRJ，或将它们打包为 ZIP");
  const projection = decode(prj).trim();
  if (!projection) throw new Error("PRJ 投影定义为空");
  // shpjs otherwise silently ignores invalid PRJ and treats projected coordinates as degrees.
  try { proj4(projection); } catch { throw new Error("无法识别 PRJ 投影，请导出有效的投影定义"); }
  const geometries = parseShp(new Uint8Array(shp).buffer, projection);
  return parseRegion(JSON.stringify({ type: "GeometryCollection", geometries }));
}

function unpack(bytes: Uint8Array, kind: "kmz" | "zip") {
  let total = 0;
  const entries = unzipSync(bytes, { filter: (entry) => {
    if (entry.name.startsWith("__MACOSX/") || entry.name.endsWith("/")) return false;
    const ext = extension(entry.name);
    if (!(kind === "kmz" ? ext === ".kml" : [".shp", ".prj"].includes(ext))) return false;
    total += entry.originalSize;
    if (total > expandedLimit) throw new Error("解压后的边界文件不能超过 64 MiB");
    return true;
  } });
  return Object.entries(entries);
}

export async function parseBoundaryFiles(files: File[]): Promise<Region> {
  if (!files.length) throw new Error("请选择边界文件");
  if (files.reduce((total, file) => total + file.size, 0) > inputLimit) throw new Error("边界文件总大小不能超过 20 MiB");
  const shapes = files.filter((file) => extension(file.name) === ".shp");
  if (shapes.length) {
    if (shapes.length !== 1 || files.some((file) => ![".shp", ".prj", ".shx", ".dbf", ".cpg"].includes(extension(file.name)) || stem(file.name) !== stem(shapes[0].name))) throw new Error("每次请选择一套同名 Shapefile 文件");
    const projections = files.filter((file) => extension(file.name) === ".prj");
    if (projections.length > 1) throw new Error("存在重复的 PRJ 文件");
    return shapeRegion(new Uint8Array(await shapes[0].arrayBuffer()), projections[0] ? new Uint8Array(await projections[0].arrayBuffer()) : undefined);
  }
  if (files.length !== 1) throw new Error("每次导入一个边界文件；Shapefile 可同时选择同名配套文件");
  const file = files[0];
  const ext = extension(file.name);
  const bytes = new Uint8Array(await file.arrayBuffer());
  if ([".geojson", ".json"].includes(ext)) return parseRegion(decode(bytes));
  if (ext === ".kml") return parseKml(decode(bytes));
  if (ext === ".kmz") {
    const entries = unpack(bytes, "kmz");
    const docs = entries.filter(([name]) => name.toLowerCase() === "doc.kml");
    const selected = docs.length === 1 ? docs[0] : entries.length === 1 ? entries[0] : null;
    if (!selected) throw new Error("KMZ 应包含 doc.kml 或唯一的 KML 边界文件");
    return parseKml(decode(selected[1]));
  }
  if (ext === ".zip") {
    const entries = unpack(bytes, "zip");
    const shp = entries.filter(([name]) => extension(name) === ".shp");
    if (shp.length !== 1) throw new Error("ZIP 必须只包含一套 Shapefile，请拆分多个图层后导入");
    const prj = entries.filter(([name]) => extension(name) === ".prj" && stem(name) === stem(shp[0][0]));
    if (prj.length > 1) throw new Error("ZIP 内存在重复的 PRJ 文件");
    return shapeRegion(shp[0][1], prj[0]?.[1]);
  }
  throw new Error("支持 GeoJSON、KML、KMZ 和 Shapefile（ZIP 或同时选择 SHP / PRJ）");
}
