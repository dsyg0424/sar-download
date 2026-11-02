
export function MapLayerControls({ vectors, onVectorsChange }: {
  vectors: boolean;
  onVectorsChange: (value: boolean) => void;
}) {
  return <div className="map-header" role="group" aria-label="地图图层">
    <button type="button" className="map-vector-toggle" aria-pressed={vectors} onClick={() => onVectorsChange(!vectors)} title="显示地名、道路与行政边界">
      <span>地名与道路</span><i className="toggle-track" aria-hidden="true" />
    </button>
  </div>;
}
