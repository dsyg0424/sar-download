import { useEffect, useState, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CircleHelp, Copy, Minus, Settings2, Square, X } from "lucide-react";
import appIcon from "../../../src-tauri/icons/32x32.png";

export function Titlebar({ onSettings, onHelp, downloads, onError }: {
  onSettings: () => void; onHelp: () => void; downloads: ReactNode; onError: (error: string) => void;
}) {
  const [appWindow] = useState(() => isTauri() ? getCurrentWindow() : null);
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!appWindow) return;
    let disposed = false; let unlisten: (() => void) | undefined;
    const sync = async () => { try { const value = await appWindow.isMaximized(); if (!disposed) setMaximized(value); } catch (e) { if (!disposed) onError(String(e)); } };
    void appWindow.onResized(() => void sync()).then((stop) => { if (disposed) stop(); else { unlisten = stop; void sync(); } }).catch((e) => { if (!disposed) onError(String(e)); });
    return () => { disposed = true; unlisten?.(); };
  }, [appWindow, onError]);
  const run = async (action: () => Promise<unknown>) => { try { await action(); } catch (e) { onError(`窗口操作失败：${String(e)}`); } };
  return <header className="custom-titlebar" aria-label="应用标题栏">
    <div className="titlebar-brand" data-tauri-drag-region>
      <img src={appIcon} alt="" draggable={false} />
      <h1 data-tauri-drag-region title="地枢遥感 SAR数据可视化下载平台">地枢遥感 SAR数据可视化下载平台</h1>
    </div>
    <div className="titlebar-drag-area" data-tauri-drag-region aria-hidden="true" />
    <nav className="titlebar-tools" aria-label="应用操作">{downloads}<button type="button" onClick={onSettings} title="下载设置" aria-label="设置"><Settings2 size={15} /><span>设置</span></button><button type="button" onClick={onHelp} title="使用帮助" aria-label="帮助"><CircleHelp size={15}/><span>帮助</span></button></nav>
    <div className="window-controls" aria-label="窗口控制">
      <button disabled={!appWindow} aria-label="最小化" title="最小化" onClick={() => appWindow && void run(() => appWindow.minimize())}><Minus size={15} /></button>
      <button disabled={!appWindow} aria-label={maximized ? "还原窗口" : "最大化"} title={maximized ? "还原窗口" : "最大化"} onClick={() => appWindow && void run(async () => { await appWindow.toggleMaximize(); setMaximized(await appWindow.isMaximized()); })}>{maximized ? <Copy size={13} /> : <Square size={13} />}</button>
      <button disabled={!appWindow} className="window-close" aria-label="关闭窗口" title="关闭窗口" onClick={() => appWindow && void run(() => appWindow.close())}><X size={17} /></button>
    </div>
  </header>;
}
