import { useId } from "react";
import { ChevronRight, Folder, FolderOpen } from "lucide-react";
import { SarSceneRow } from "./SarSceneRow";
import type { useSarResults } from "../hooks/useSarResults";

type ResultsModel = ReturnType<typeof useSarResults>;
export function SarPathGroup({ group, model }: { group: ResultsModel["groups"][number]; model: ResultsModel }) {
  const { store, keysFor, toggleKeys, expandedGroups, setExpandedGroups, expandedProducts, setExpandedProducts, productKey, toggleExpanded, groupElements } = model;
  const childrenId = useId();
  const expanded = expandedGroups.has(group.id);
  const keys = keysFor(group);
  return <li ref={(element) => { if (element) groupElements.current.set(group.id, element); else groupElements.current.delete(group.id); }}
    className={`sar-path-group${store.activeGroup === group.id ? " active" : ""}`}>
    <div className="sar-path-heading">
      <label className="sar-check-cell"><input type="checkbox" aria-label={`选择分组 ${group.label}`}
        checked={keys.every((key) => store.selected.includes(key))}
        ref={(element) => { if (element) element.indeterminate = keys.some((key) => store.selected.includes(key)) && !keys.every((key) => store.selected.includes(key)); }}
        onChange={() => toggleKeys(keys)} /></label>
      <button type="button" className="sar-path-title" aria-label={`${expanded ? "折叠" : "展开"}分组 ${group.label}`}
        aria-pressed={store.activeGroup === group.id} aria-expanded={expanded} aria-controls={expanded ? childrenId : undefined}
        onClick={() => { setExpandedGroups((values) => toggleExpanded(values, group.id)); store.activate(group.id, true); }}>
        <ChevronRight size={12} className={`sar-disclosure${expanded ? " expanded" : ""}`} aria-hidden="true" />
        {expanded ? <FolderOpen size={14} className="sar-folder-icon" aria-hidden="true" /> : <Folder size={14} className="sar-folder-icon" aria-hidden="true" />}
        <span>{group.label}</span><small className="sar-group-count">{group.products.length}</small>
      </button>
    </div>
    {expanded && <ul id={childrenId} className="sar-path-children" aria-label={`${group.label} 的产品类型`}>
      {[...new Set(group.products.map((item) => item.product))].map((productType) => {
        const products = group.products.filter((item) => item.product === productType);
        const ids = products.map((item) => item.id);
        const key = productKey(group.id, productType);
        const productExpanded = expandedProducts.has(key);
        const scenesId = `${childrenId}-${encodeURIComponent(productType)}`;
        return <li className="sar-product-group" key={key}>
          <div className="sar-product-heading">
            <label className="sar-check-cell"><input type="checkbox" aria-label={`选择 ${group.label} 中的 ${productType} 影像`}
              checked={ids.every((id) => store.selected.includes(id))}
              ref={(element) => { if (element) element.indeterminate = ids.some((id) => store.selected.includes(id)) && !ids.every((id) => store.selected.includes(id)); }}
              onChange={() => toggleKeys(ids)} /></label>
            <button type="button" className="sar-product-title" aria-label={`${productExpanded ? "折叠" : "展开"} ${group.label} 中的 ${productType}`}
              aria-expanded={productExpanded} aria-controls={productExpanded ? scenesId : undefined}
              onClick={() => setExpandedProducts((values) => toggleExpanded(values, key))}>
              <ChevronRight size={12} className={`sar-disclosure${productExpanded ? " expanded" : ""}`} aria-hidden="true" />
              {productExpanded ? <FolderOpen size={14} className="sar-folder-icon" aria-hidden="true" /> : <Folder size={14} className="sar-folder-icon" aria-hidden="true" />}
              <span>{productType}</span><small className="sar-product-count">（{products.length}）</small>
            </button>
          </div>
          {productExpanded && <ul id={scenesId} className="sar-product-children" aria-label={`${group.label} ${productType} 影像`}>
            {products.map((item) => <SarSceneRow key={item.id} item={item} model={model} />)}
          </ul>}
        </li>;
      })}
    </ul>}
  </li>;
}
