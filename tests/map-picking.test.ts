import { afterEach, expect, test } from "bun:test";
import type { Map } from "maplibre-gl";
import { bindScenePicking } from "../src/features/map/scenePicking";
import { useSarSearchStore } from "../src/features/sar-download/store/useSarSearchStore";
import type { SarProduct } from "../src/features/sar-download/services/sarSearchApi";

const product = (id: string): SarProduct => ({ id, granule: id, name: id, product: "GSLC", url: "https://example.invalid/a", size: 1, date: "2026-06-01", platform: "NISAR", polarization: "HH", direction: "ASCENDING", relativeOrbit: 1, geometry: {type:"Polygon",coordinates:[[[0,0],[1,0],[1,1],[0,0]]]} });
afterEach(() => { useSarSearchStore.setState({busy:false});useSarSearchStore.getState().reset(); });

function mapFixture() {
  let listener: ((event: any) => void) | undefined;
  let features: any[] = [];
  let layer = true;
  let queries = 0;
  const map = {
    on: (_name: string,callback: (event:any)=>void) => {listener=callback;},
    off: (_name: string,callback: (event:any)=>void) => {if(listener===callback)listener=undefined;},
    getLayer: () => layer,
    queryRenderedFeatures: (_point:unknown, options:{layers:string[]}) => { expect(options.layers).toEqual(["footprints"]);queries++;return features; },
  } as unknown as Map;
  return {map,click:()=>listener?.({point:{x:4,y:5}}),hit:(id?:string)=>{features=id?[{properties:{id}}]:[];},hideLayer:()=>{layer=false;},queries:()=>queries};
}

test("a canvas background click clears the scene/group mark and keeps download checkboxes", () => {
  const fixture=mapFixture(); const stop=bindScenePicking(fixture.map,()=>true);
  useSarSearchStore.setState({results:[product("first"),product("second")],selected:["first"]});
  fixture.hit("first");fixture.click();expect(useSarSearchStore.getState().activeScene).toBe("first");
  fixture.hit("second");fixture.click();expect(useSarSearchStore.getState().activeScene).toBe("second");
  fixture.hit();fixture.click();
  expect(useSarSearchStore.getState().activeScene).toBeNull();expect(useSarSearchStore.getState().activeGroup).toBeNull();
  expect(useSarSearchStore.getState().selected).toEqual(["first"]);expect(useSarSearchStore.getState().focusRequest).toBeNull();
  stop();fixture.hit("first");fixture.click();expect(useSarSearchStore.getState().activeScene).toBeNull();
});

test("drawing and its suppressed completion click never clear or change a scene mark", () => {
  const fixture=mapFixture();let enabled=true;bindScenePicking(fixture.map,()=>enabled);
  useSarSearchStore.setState({results:[product("first")]});fixture.hit("first");fixture.click();
  enabled=false;fixture.hit();fixture.click();expect(useSarSearchStore.getState().activeScene).toBe("first");
  enabled=true;fixture.hideLayer();fixture.click();expect(useSarSearchStore.getState().activeScene).toBe("first");
  expect(fixture.queries()).toBe(1);
});
