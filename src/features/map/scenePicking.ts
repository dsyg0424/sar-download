import type { Map } from "maplibre-gl";
import { useSarSearchStore } from "../sar-download/store/useSarSearchStore";

export function bindScenePicking(map: Map, enabled: () => boolean): () => void {
  const pick = (event: { point: Parameters<Map["queryRenderedFeatures"]>[0] }) => {
    if (!enabled() || !map.getLayer("footprints")) return;
    const feature = map.queryRenderedFeatures(event.point, { layers: ["footprints"] })[0];
    const id = feature?.properties?.id;
    const store = useSarSearchStore.getState();
    if (id != null) store.activateScene(String(id));
    else store.clearActiveScene();
  };
  map.on("click", pick);
  return () => { map.off("click", pick); };
}
