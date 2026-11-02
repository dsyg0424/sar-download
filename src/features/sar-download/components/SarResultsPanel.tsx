import { SarPathGroup } from "./SarPathGroup";
import { useSarSearchStore } from "../store/useSarSearchStore";
import type { useSarResults } from "../hooks/useSarResults";

export function SarResultsPanel({
  model,
  adding,
  exporting = false,
  onAdd,
  onImport,
}: {
  model: ReturnType<typeof useSarResults>;
  adding: boolean;
  exporting?: boolean;
  onAdd: () => void;
  onImport: () => void;
}) {
  const {
    store,
    groups,
    allKeys,
    products,
    expandAll,
    collapseAll,
    resultViewport,
  } = model;
  return (
    <section className="sar-results-panel" aria-label="检索结果">
      <div className="sar-results-toolbar">
        <label className="sar-select-all">
          <input
            type="checkbox"
            aria-label="选择全部检索结果"
            checked={allKeys.every((key) => store.selected.includes(key))}
            ref={(element) => {
              if (element)
                element.indeterminate =
                  allKeys.some((key) => store.selected.includes(key)) &&
                  !allKeys.every((key) => store.selected.includes(key));
            }}
            onChange={(e) =>
              useSarSearchStore.setState({
                selected: e.target.checked ? allKeys : [],
              })
            }
          />
          全选
        </label>
        <span className="sar-results-count">
          {store.results.length} 景 · 已选 {products.length}
        </span>
        <button
          type="button"
          onClick={expandAll}
        >
          全部展开
        </button>
        <button type="button" onClick={collapseAll}>
          全部折叠
        </button>
      </div>
      <div className="sar-results" ref={resultViewport}>
        <ul className="sar-results-tree" aria-label="影像目录">
        {groups.map((group) => (
          <SarPathGroup key={group.id} group={group} model={model} />
        ))}
        </ul>
      </div>
      <div className="sar-result-actions">
        <button className="btn btn-secondary" disabled={!products.length || adding || exporting} onClick={onImport}>
          {exporting ? "导出中…" : "导出所选 GeoJSON"}
        </button>
        <button
          className="btn btn-primary"
          disabled={!products.length || adding}
          onClick={onAdd}
        >
          {adding ? "请稍候…" : `下载所选（${products.length} 景）`}
        </button>
      </div>
    </section>
  );
}
