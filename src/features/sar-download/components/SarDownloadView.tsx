import { useMemo, useRef, useState, type ReactNode } from "react";
import { sarQueryKey } from "../model/searchOptions";
import { useSarSearchStore } from "../store/useSarSearchStore";
import type { SarProduct } from "../services/sarSearchApi";
import { useSarResults } from "../hooks/useSarResults";
import { SarSearchForm } from "./SarSearchForm";
import { SarSearchProgress } from "./SarSearchProgress";
import { SarResultsPanel } from "./SarResultsPanel";
import "../styles/sar-download.css";

export function SarDownloadView({ areaSelector, onReset, onDownload, onImport, active = true }: {
  areaSelector: ReactNode;
  onReset: () => void;
  onDownload: (products: SarProduct[]) => Promise<void>;
  onImport: (products: SarProduct[]) => Promise<void>;
  active?: boolean;
}) {
  const store = useSarSearchStore();
  const [adding, setAdding] = useState(false);
  const [exporting, setExporting] = useState(false);
  const exportLock = useRef(false);
  const [actionError, setActionError] = useState("");
  const results = useSarResults(active);
  const { products } = results;
  const stale = useMemo(() => store.resultRequest !== null && sarQueryKey(store.request) !== sarQueryKey(store.resultRequest), [store.request, store.resultRequest]);
  const noResults = store.searched && !store.busy && !store.error && !store.results.length;
  const hasFeedback = store.error || store.busy || stale || store.warnings.length > 0 || noResults || actionError;
  const add = async () => {
    setAdding(true); setActionError("");
    try { await onDownload(products); }
    catch (error) { setActionError(String(error)); }
    finally { setAdding(false); }
  };
  const exportSelected = async () => {
    if (exportLock.current) return;
    exportLock.current = true; setExporting(true); setActionError("");
    try { await onImport([...products]); }
    catch (error) { setActionError(String(error)); }
    finally { exportLock.current = false; setExporting(false); }
  };
  return <div className={`sar-download-view${store.results.length ? " has-results" : ""}`}>
    <section className="search-dock glass-panel" aria-labelledby="sar-search-heading">
      <header className="dock-heading"><h2 id="sar-search-heading">影像检索</h2></header>
      <SarSearchForm areaSelector={areaSelector} adding={adding} onReset={() => { onReset(); setActionError(""); }} />
    </section>
    {hasFeedback && <div className="search-feedback glass-panel">
      {store.error && <p className="sar-error" role="alert">{store.error}</p>}
      <SarSearchProgress />
      {stale && <p className="sar-note" role="status">条件已修改，结果仍为上次检索。</p>}
      {store.warnings.map((warning) => <p key={warning} className="sar-error" role="alert">{warning}</p>)}
      {noResults && <p className="sar-note" role="status">未找到符合条件的影像。</p>}
      {actionError && <p className="sar-error" role="alert">{actionError}</p>}
    </div>}
    {store.results.length > 0 && <SarResultsPanel model={results} adding={adding} exporting={exporting} onAdd={() => void add()}
      onImport={() => void exportSelected()} />}
  </div>;
}
