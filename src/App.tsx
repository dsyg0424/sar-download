import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import { Titlebar } from "./shared/components/Titlebar";
import { SarDownloadView } from "./features/sar-download/components/SarDownloadView";
import { useSarSearchStore } from "./features/sar-download/store/useSarSearchStore";
import type { SarProduct } from "./features/sar-download/services/sarSearchApi";
import { localDate } from "./shared/utils/localDate";
import { RegionSelector } from "./features/map/RegionSelector";
import { DownloadCenter } from "./features/downloads/DownloadCenter";
import { SettingsDialog } from "./features/downloads/SettingsDialog";
import { defaults, downloadApi, mergeTask, type DownloadTask, type Settings } from "./features/downloads/api";
import "maplibre-gl/dist/maplibre-gl.css";
import "./App.css";
import { HelpDialog } from "./features/help/HelpDialog";
import { useHelp } from "./features/help/useHelp";
import userGuide from "../docs/新用户操作指南.md?raw";

import { finishSubmission } from "./features/downloads/submission";
import { SnapshotGate } from "./features/downloads/snapshotGate";

const desktop = isTauri();
const SearchMap = lazy(() => import("./features/map/SearchMap").then((module) => ({ default: module.SearchMap })));
function App() {
  const help = useHelp();
  const [settings, setSettings] = useState<Settings>(defaults);
  const removedTaskIds = useRef(new Set<string>());
  const snapshotGate = useRef(new SnapshotGate());
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pending, setPending] = useState<Set<string>>(new Set());
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [settingsLoading, setSettingsLoading] = useState(desktop);
  const [downloadRevealVersion, setDownloadRevealVersion] = useState(0);
  const refresh = useCallback(async () => {
    const ticket = snapshotGate.current.begin();
    const snapshot = await downloadApi.list();
    setTasks((old) => snapshotGate.current.merge(old, snapshot, ticket, removedTaskIds.current));
  }, []);
  useEffect(() => {
    const search = useSarSearchStore.getState();
    if (!search.request.start) {
      const start = new Date(); start.setDate(start.getDate() - 30);
      search.update({ start: localDate(start), end: localDate() });
    }
    if (!desktop) return;
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const attach = async <T,>(name: string, callback: (value: T) => void) => {
      const unlisten = await listen<T>(name, (event) => { if (!disposed) callback(event.payload); });
      if (disposed) unlisten(); else unlisteners.push(unlisten);
    };
    void (async () => {
      const results = await Promise.allSettled([
        attach<DownloadTask>("download-progress", (task) => { if (!removedTaskIds.current.has(task.id)) { snapshotGate.current.touch(task.id); setTasks((old) => mergeTask(old, task)); } }),
        attach<string>("download-error", setError),
        downloadApi.loadSettings(),
        refresh(),
      ] as const);
      if (disposed) return;
      const warnings: string[] = [];
      for (const result of results.slice(0, 2)) if (result.status === "rejected") warnings.push(String(result.reason));
      const configuration = results[2];
      if (configuration.status === "fulfilled") {
        setSettings(configuration.value); setLoaded(true);
        if (configuration.value.credentialWarning) warnings.push(configuration.value.credentialWarning);
      } else warnings.push(`设置读取失败：${String(configuration.reason)}`);
      const snapshot = results[3];
      if (snapshot.status === "rejected") warnings.push(`任务恢复失败：${String(snapshot.reason)}`);
      setSettingsLoading(false);
      if (warnings.length) setError(warnings.join("；"));
    })();
    const polling = window.setInterval(() => { if (!disposed) void refresh().catch((e) => { if (!disposed) setError(String(e)); }); }, 10000);
    return () => { disposed = true; snapshotGate.current.invalidate(); unlisteners.forEach((fn) => fn()); window.clearInterval(polling); };
  }, [refresh]);

  const action = async (operation: "start" | "pause" | "remove", ids: string[]) => {
    if (operation === "start" && (!settings.username || !settings.password)) { setError("请先在下载设置中填写 Earthdata 账号和密码。"); setSettingsOpen(true); return; }
    setPending((old) => new Set([...old, ...ids])); setError("");
    const errors: string[] = [];
    if (operation === "remove") {
      try {
        await downloadApi.removeMany(ids);
        ids.forEach((id) => removedTaskIds.current.add(id));
        setTasks((old) => old.filter((task) => !removedTaskIds.current.has(task.id)));
      } catch (error) { errors.push(String(error)); }
    } else {
      for (const id of ids) {
        try {
          if (operation === "start") await downloadApi.start(id, settings);
          else await downloadApi.pause(id);
        } catch (error) { errors.push(String(error)); }
      }
    }
    try { await refresh(); } catch (e) { errors.push(String(e)); }
    setPending((old) => { const next = new Set(old); ids.forEach((id) => next.delete(id)); return next; });
    if (errors.length) setError([...new Set(errors)].join("；"));
  };
  const add = async (products: SarProduct[]) => {
    if (!desktop) throw new Error("请在桌面应用中下载影像。");
    if (!loaded || !settings.outputDir) { setSettingsOpen(true); throw new Error("请先设置下载目录。"); }
    const result = await downloadApi.add(products, settings);
    result.added.forEach((id) => removedTaskIds.current.delete(id));
    setNotice(`已加入 ${result.added.length} 个任务${result.skipped ? `，跳过 ${result.skipped} 个重复任务` : ""}。`);
    if (result.added.length) {
      setDownloadRevealVersion((value) => value + 1);
    }
    await finishSubmission(result.added, ids => action("start", ids), refresh,
      error => setError(previous => [previous, `任务已创建，状态同步失败：${String(error)}`].filter(Boolean).join("；")));
  };
  const open = (path: string) => { void downloadApi.openDirectory(path).catch((e) => setError(String(e))); };
  return <div className="app-shell">
    <Titlebar onHelp={help.show} onSettings={() => setSettingsOpen(true)} downloads={<DownloadCenter tasks={tasks} pending={pending} onAction={action} onOpen={open} revealVersion={downloadRevealVersion} cleanup={desktop ? {preview:()=>downloadApi.previewCleanup(settings.outputDir),execute:downloadApi.executeCleanup} : undefined} />} onError={setError} />
    {!desktop && <div className="preview-banner" role="note">网页预览 · 检索和下载请使用桌面应用。</div>}
    <div className="app-notifications">
      {error && <div className="app-message error-message" role="alert"><span>{error}</span><button aria-label="关闭错误提示" onClick={() => setError("")}><X size={15} /></button></div>}
      {notice && <div className="app-message notice-message" role="status"><span>{notice}</span><button aria-label="关闭操作提示" onClick={() => setNotice("")}><X size={15} /></button></div>}
    </div>
    <main className="workspace" aria-label="SAR 检索与下载">
      <Suspense fallback={<section className="search-map map-pending" aria-label="地图加载中" />}><SearchMap /></Suspense>
      <aside className="left-hud" aria-label="影像检索与结果">
        <SarDownloadView areaSelector={<RegionSelector />} onReset={() => useSarSearchStore.getState().reset()} onDownload={add}
          onImport={async (products) => { const path = await invoke<string | null>("export_results", { products }); if (path) setNotice(`已导出至 ${path}`); }} />
      </aside>
    </main>
    {help.open && <HelpDialog source={userGuide} onClose={help.close}/>}
    {settingsOpen && <SettingsDialog settings={settings} loading={settingsLoading} onClose={() => setSettingsOpen(false)} onSave={async (next) => {
      const saved = await downloadApi.saveSettings(next); setSettings(saved); setLoaded(true); setNotice("下载设置已保存。");
    }} />}
  </div>;
}
export default App;
