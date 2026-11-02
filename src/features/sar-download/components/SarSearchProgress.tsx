import { stageLabels } from "../model/searchProgress";
import { useSarSearchStore } from "../store/useSarSearchStore";

export function SarSearchProgress() {
  const busy = useSarSearchStore((s) => s.busy);
  const progress = useSarSearchStore((s) => s.searchProgress);
  if (!busy) return null;
  const entries = Object.entries(progress);
  const completed = entries.filter(([, p]) => p.complete).length;
  return (
    <div className="sar-note" role="status" aria-live="polite">
      <p>
        检索中 · {completed}/{entries.length || "—"} 类完成
      </p>
      {entries.map(([product, p]) => (
        <span key={product} className="sar-query-product">
          {product}：
          {p.complete
            ? `${p.count} 景，已完成`
            : `${p.count ? `已显示 ${p.count} 景，` : ""}${stageLabels[p.stage] ?? "等待目录响应"}`}
        </span>
      ))}
    </div>
  );
}
