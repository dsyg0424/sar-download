import { FileText, Info } from "lucide-react";
import type { SarProduct } from "../services/sarSearchApi";
import type { useSarResults } from "../hooks/useSarResults";

export function SarSceneRow({
  item,
  model,
}: {
  item: SarProduct;
  model: ReturnType<typeof useSarResults>;
}) {
  const {
    store,
    toggleKeys,
    groupElements,
    expandedScenes,
    setExpandedScenes,
    toggleExpanded,
  } = model;
  return (
    <li className="sar-scene-item">
      <div
        className={`sar-scene-row${store.activeScene === item.id ? " active" : ""}`}
      >
        <input
          type="checkbox"
          aria-label={`选择影像 ${item.granule}`}
          checked={store.selected.includes(item.id)}
          onChange={() => toggleKeys([item.id])}
        />
        <button
          type="button"
          ref={(element) => {
            if (element) groupElements.current.set(item.id, element);
            else groupElements.current.delete(item.id);
          }}
          className={`sar-path-scene${store.activeScene === item.id ? " active" : ""}`}
          aria-pressed={store.activeScene === item.id}
          aria-label={`定位影像 ${item.granule}`}
          onClick={() => store.activateScene(item.id, true)}
          title="定位影像"
        >
          <FileText size={13} className="sar-file-icon" aria-hidden="true" />
          <time dateTime={item.date}>
            {item.date.replace("T", " ").slice(0, 19)}
          </time>
          <span className="sar-scene-polarization">
            {item.polarization || "—"}
          </span>
        </button>
        <button
          type="button"
          className="sar-icon-button"
          aria-label={`${expandedScenes.has(item.id) ? "收起" : "查看"} ${item.granule} 详情`}
          title="影像详情"
          aria-expanded={expandedScenes.has(item.id)}
          onClick={() =>
            setExpandedScenes((values) => toggleExpanded(values, item.id))
          }
        >
          <Info size={14} />
        </button>
      </div>
      {expandedScenes.has(item.id) && (
        <dl className="sar-scene-details">
          <dt>影像标识</dt>
          <dd>{item.granule}</dd>
          <dt>卫星</dt>
          <dd>{item.platform || "—"}</dd>
          <dt>采集时间</dt>
          <dd>{item.date}（UTC）</dd>
          <dt>文件名</dt>
          <dd>{item.name || "—"}</dd>
          <dt>文件大小</dt>
          <dd>
            {item.size > 0 ? `${item.size.toLocaleString()} 字节` : "未知"}
          </dd>
        </dl>
      )}
    </li>
  );
}
