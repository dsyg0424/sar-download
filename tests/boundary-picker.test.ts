import { expect, test } from "bun:test";
import { decodeBoundarySelection } from "../src/features/map/boundaryPicker";

function payload(parts: { name: string; size: number }[], content: string) {
  const header = new TextEncoder().encode(JSON.stringify(parts));
  const bytes = new TextEncoder().encode(content);
  const result = new Uint8Array(4 + header.length + bytes.length);
  new DataView(result.buffer).setUint32(0,header.length,true);
  result.set(header,4);result.set(bytes,4+header.length);
  return result.buffer;
}

test("native SHP selection retains automatically discovered PRJ and Unicode file names", async () => {
  const files = decodeBoundarySelection(payload([{name:"边界.SHP",size:5},{name:"边界.PRJ",size:4}],"shapeproj"))!;
  expect(files.map(file=>file.name)).toEqual(["边界.SHP","边界.PRJ"]);
  expect(await files[0].text()).toBe("shape");expect(await files[1].text()).toBe("proj");
});

test("cancelled native file picker returns no replacement boundary", () => {
  expect(decodeBoundarySelection(new ArrayBuffer(0))).toBeNull();
});

test("truncated and invalid native file responses cannot enter the parser", () => {
  expect(()=>decodeBoundarySelection(new ArrayBuffer(2))).toThrow();
  expect(()=>decodeBoundarySelection(payload([{name:"a.shp",size:6}],"short"))).toThrow();
  expect(()=>decodeBoundarySelection(payload([{name:"a.shp",size:-1}],""))).toThrow();
  expect(()=>decodeBoundarySelection(payload([{name:"a.shp",size:2}],"trailing"))).toThrow();
});
