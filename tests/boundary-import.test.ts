import { expect, test } from "bun:test";
import { zipSync, strToU8 } from "fflate";
import proj4 from "proj4";
import { parseBoundaryFiles } from "../src/features/map/boundaryParser";
import { regionBounds } from "../src/features/map/region";

const outer = [[100,20],[100,21],[101,21],[101,20],[100,20]];
const hole = [[100.2,20.2],[100.8,20.2],[100.8,20.8],[100.2,20.8],[100.2,20.2]];
const geo = { type: "Polygon", coordinates: [outer, hole] };
const prj = 'GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]';
const mercator = 'PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1],EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0.0 +lon_0=0.0 +x_0=0.0 +y_0=0 +k=1.0 +units=m +nadgrids=@null +wktext +no_defs"]]';
const kml = `<?xml version="1.0"?><kml xmlns="http://www.opengis.net/kml/2.2"><Document><Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>${outer.map(p=>p.join(",")).join(" ")}</coordinates></LinearRing></outerBoundaryIs><innerBoundaryIs><LinearRing><coordinates>${hole.map(p=>p.join(",")).join(" ")}</coordinates></LinearRing></innerBoundaryIs></Polygon></Placemark></Document></kml>`;
const file = (name: string, data: string | Uint8Array) => new File([typeof data === "string" ? data : new Uint8Array(data).buffer], name);

// A real ESRI Polygon record, including two parts; no parser mocks.
function polygonShp(rings: number[][][]): Uint8Array {
  const points = rings.flat(); const contentSize = 44 + rings.length*4 + points.length*16;
  const bytes = new Uint8Array(108+contentSize); const view = new DataView(bytes.buffer);
  const bounds = [Math.min(...points.map(p=>p[0])),Math.min(...points.map(p=>p[1])),Math.max(...points.map(p=>p[0])),Math.max(...points.map(p=>p[1]))];
  view.setInt32(0,9994); view.setInt32(24,bytes.length/2); view.setInt32(28,1000,true); view.setInt32(32,5,true);
  bounds.forEach((v,i)=>view.setFloat64(36+i*8,v,true));
  view.setInt32(100,1);view.setInt32(104,contentSize/2);view.setInt32(108,5,true);
  bounds.forEach((v,i)=>view.setFloat64(112+i*8,v,true));
  view.setInt32(144,rings.length,true);view.setInt32(148,points.length,true);
  let start = 0; rings.forEach((ring,i)=>{view.setInt32(152+i*4,start,true);start+=ring.length;});
  points.forEach((p,i)=>{view.setFloat64(152+rings.length*4+i*16,p[0],true);view.setFloat64(160+rings.length*4+i*16,p[1],true);});
  return bytes;
}

test("GeoJSON, KML and KMZ retain identical bounds and polygon holes", async () => {
  for (const input of [file("area.geojson",JSON.stringify(geo)),file("area.kml",kml),file("area.kmz",zipSync({"doc.kml":strToU8(kml),"icon.png":new Uint8Array([1,2])}))]) {
    const result = await parseBoundaryFiles([input]);
    expect(result.type).toBe("Polygon");expect(result.coordinates.length).toBe(2);
    expect(regionBounds(result)).toEqual([100,20,101,21]);
  }
});

test("standalone Shapefile companions and nested uppercase ZIP preserve holes", async () => {
  const shp = polygonShp([outer,hole]);
  const sets = [[file("Area.SHP",shp),file("Area.PRJ",prj)], [file("area.zip",zipSync({"folder/Area.SHP":shp,"folder/Area.PRJ":strToU8(prj)}))]];
  for (const files of sets) {
    const result = await parseBoundaryFiles(files);
    expect(result.coordinates.length).toBe(2);expect(regionBounds(result)).toEqual([100,20,101,21]);
  }
});

test("projected Shapefile coordinates really transform to WGS84", async () => {
  const shp = polygonShp([outer.map(p=>proj4("EPSG:4326","EPSG:3857",p))]);
  const bounds = regionBounds(await parseBoundaryFiles([file("projected.shp",shp),file("projected.prj",mercator)]));
  [100,20,101,21].forEach((value,i)=>expect(bounds[i]).toBeCloseTo(value,7));
});

test("missing and invalid PRJ are rejected even when source coordinates look like degrees", async () => {
  const shp = file("area.shp",polygonShp([outer]));
  await expect(parseBoundaryFiles([shp])).rejects.toThrow("缺少同名 PRJ");
  await expect(parseBoundaryFiles([shp,file("area.prj","not a projection")])).rejects.toThrow("无法识别 PRJ");
  await expect(parseBoundaryFiles([file("area.zip",zipSync({"area.shp":polygonShp([outer])}))])).rejects.toThrow("缺少同名 PRJ");
});

test("ambiguous archives and multiple unrelated files require explicit separation", async () => {
  await expect(parseBoundaryFiles([file("area.zip",zipSync({"a.shp":polygonShp([outer]),"b.shp":polygonShp([outer])}))])).rejects.toThrow("拆分多个图层");
  await expect(parseBoundaryFiles([file("area.kmz",zipSync({"a.kml":strToU8(kml),"b.kml":strToU8(kml)}))])).rejects.toThrow("唯一的 KML");
  await expect(parseBoundaryFiles([file("a.json",JSON.stringify(geo)),file("b.json",JSON.stringify(geo))])).rejects.toThrow("每次导入一个");
});

test("KML MultiGeometry polygons combine without losing holes", async () => {
  const polygon = kml.match(/<Polygon>.*<\/Polygon>/)![0];
  const multi = kml.replace(polygon,`<MultiGeometry>${polygon}${polygon}</MultiGeometry>`);
  const result = await parseBoundaryFiles([file("multi.kml",multi)]);
  expect(result.type).toBe("MultiPolygon");expect(result.coordinates.length).toBe(2);
  expect(result.coordinates[0].length).toBe(2);
});

test("point/line, open rings, projected GeoJSON and malformed XML cannot become search boundaries", async () => {
  for (const value of [{type:"Point",coordinates:[100,20]},{type:"Polygon",coordinates:[outer.slice(0,4)]},{...geo,crs:{properties:{name:"EPSG:3857"}}}]) {
    await expect(parseBoundaryFiles([file("bad.geojson",JSON.stringify(value))])).rejects.toThrow();
  }
  await expect(parseBoundaryFiles([file("bad.kml","<kml><Polygon></kml>")])).rejects.toThrow();
  await expect(parseBoundaryFiles([file("bad.kml",'<!DOCTYPE kml><kml/>')])).rejects.toThrow("DTD");
  await expect(parseBoundaryFiles([file("old.gpkg","not supported")])).rejects.toThrow("支持 GeoJSON");
});

test("input and decompressed boundary limits are enforced", async () => {
  await expect(parseBoundaryFiles([file("large.json",new Uint8Array(20*1024*1024+1))])).rejects.toThrow("20 MiB");
  const archive = zipSync({"doc.kml":new Uint8Array(64*1024*1024+1)});
  await expect(parseBoundaryFiles([file("large.kmz",archive)])).rejects.toThrow("64 MiB");
});
