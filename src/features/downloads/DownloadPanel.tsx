import { ChevronUp, Download, FolderOpen, Pause, Play, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { TransferSpeed } from "../downloader/model/transferSpeed";
import { formatBytes, running, statusLabels, type DownloadTask } from "./api";
import type { CleanupActions } from "./api";
import { CleanupDialog } from "./CleanupDialog";

export function DownloadPanel({ tasks, pending, onAction, onOpen, onClose, cleanup, visible: expanded = true }: {
  visible?: boolean;
  tasks: DownloadTask[]; pending: Set<string>;
  onAction: (action: "start" | "pause" | "remove", ids: string[]) => Promise<void>;
  onOpen: (path: string) => void;
  onClose?: () => void;
  cleanup?: CleanupActions;
}) {
  const [cleanupOpen,setCleanupOpen]=useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState("");
  const [filter, setFilter] = useState("all");
  const samples = useRef(new Map<string, { revision: number; status: string; speed: TransferSpeed }>());
  const [displayedSpeeds, setDisplayedSpeeds] = useState<Record<string, number>>({});
  useEffect(() => {
    const ids = new Set(tasks.map((task) => task.id));
    for (const id of samples.current.keys()) if (!ids.has(id)) samples.current.delete(id);
    for (const task of tasks) {
      const sample = samples.current.get(task.id) ?? { revision: -1, status: task.status, speed: new TransferSpeed() };
      if (sample.revision !== task.revision) sample.speed.sample(task.speed_bps, performance.now());
      sample.revision = task.revision; sample.status = task.status; samples.current.set(task.id, sample);
    }
  }, [tasks]);
  const hasActive = tasks.some(task => running(task.status));
  useEffect(() => {
    if (!expanded || !hasActive) return;
    const update = () => {
      const now = performance.now();
      setDisplayedSpeeds(Object.fromEntries([...samples.current].map(([id, sample]) => [id, running(sample.status) ? sample.speed.read(now) : 0])));
    };
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [expanded, hasActive]);
  const active = tasks.filter((t) => running(t.status));
  const waiting = tasks.filter((t) => !running(t.status) && t.status !== "Completed");
  const visible = tasks.filter((t) => filter === "all" || (filter === "active" ? running(t.status) : t.status === "Completed"));
  const speed = active.reduce((sum, t) => sum + (displayedSpeeds[t.id] ?? 0), 0);
  return <section aria-busy={removing} className="downloads-panel" aria-labelledby="downloads-heading">
    <div className="panel-heading"><h2 id="downloads-heading">下载任务 <span className="count">{tasks.length}</span></h2>
      <div className="download-heading-actions">{active.length > 0 && <span className="transfer-rate" aria-label="总下载速度">{formatBytes(speed)}/s</span>}
        {onClose && <button type="button" className="icon-button" aria-label="收起下载中心" title="收起下载中心" onClick={onClose}><ChevronUp size={16} /></button>}</div></div>
    <div className="queue-toolbar">
      <div className="segmented" role="group" aria-label="下载任务筛选">{[["all", "全部"], ["active", "进行中"], ["done", "已完成"]].map(([value, label]) =>
        <button key={value} aria-pressed={filter === value} className={filter === value ? "active" : ""} onClick={() => setFilter(value)}>{label}</button>)}</div>
      <div className="queue-actions"><button disabled={!waiting.length || pending.size > 0 || removing} onClick={() => void onAction("start", waiting.map((t) => t.id))}><Play size={13} />全部开始</button>
        <button disabled={!active.length || pending.size > 0} onClick={() => void onAction("pause", active.map((t) => t.id))}><Pause size={13} />全部暂停</button>
        <button type="button" className="queue-delete-all" disabled={!tasks.length || pending.size > 0 || removing}
          title="暂停运行任务并移除全部任务记录，保留本地文件与续传数据" onClick={async () => {
            setRemoving(true); setRemoveError("");
            try { await onAction("remove", tasks.map((task) => task.id)); }
            catch (error) { setRemoveError(String(error)); }
            finally { setRemoving(false); }
          }}><Trash2 size={13} />{removing ? "清空中…" : "清空记录"}</button>
        {cleanup&&<button type="button" disabled={removing||pending.size>0} onClick={()=>setCleanupOpen(true)}>空间清理</button>}</div>
    </div>
    {removing && <p className="queue-action-status" role="status">正在停止任务并清除记录，本地文件保留。</p>}
    {removeError && <p className="queue-action-status error-message" role="alert">{removeError}</p>}
    {cleanupOpen&&cleanup&&<CleanupDialog actions={cleanup} onClose={()=>setCleanupOpen(false)}/>}
    <div className="download-list" role={visible.length ? "list" : undefined} aria-label={visible.length ? "下载任务列表" : undefined}>
      {!visible.length && <div className="empty-state" role="status"><Download size={28} strokeWidth={1.4} /><strong>{!tasks.length ? "暂无下载任务" : filter === "active" ? "暂无进行中的任务" : "暂无已完成的任务"}</strong></div>}
      {[...visible].reverse().map((task) => {
        const percent = task.total_bytes ? Math.min(task.status === "Completed" ? 100 : 99, task.downloaded_bytes / task.total_bytes * 100) : 0;
        const isRunning = running(task.status);
        const dataFormat = task.file.name.toLowerCase().endsWith(".tiff") ? "TIFF" : "ZIP";
        const progress = task.status === "Verifying" && task.total_bytes ? task.verified_bytes / task.total_bytes * 100 : percent;
        return <article className="download-item" key={task.id} role="listitem" aria-label={`${task.product} · ${task.file.name}`}>
          <div className="task-main"><strong title={[task.file.name, task.companion?.name].filter(Boolean).join("\n")}>{task.file.name}</strong>
            <span className={`task-status ${task.status.toLowerCase()}`}>{statusLabels[task.status] || task.status}</span></div>
          <div className={`task-progress ${!task.total_bytes && isRunning ? "indeterminate" : ""}`} role="progressbar" aria-label={`${task.file.name} 下载进度`} aria-valuetext={`${statusLabels[task.status] || task.status}，${formatBytes(task.downloaded_bytes)} / ${task.total_bytes ? formatBytes(task.total_bytes) : "大小未知"}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={task.total_bytes ? Math.round(progress) : undefined}><span style={{ width: `${task.status === "Completed" ? 100 : progress}%` }} /></div>
          <div className="task-details"><span>{formatBytes(task.downloaded_bytes)} / {task.total_bytes ? formatBytes(task.total_bytes) : "大小待探测"}</span>
            {task.companion && <span>{task.status === "Completed" ? `${dataFormat} + XML` : task.active_file?.endsWith(".xml") ? "XML" : dataFormat}</span>}
            <span>{task.status === "Verifying" ? `校验 ${Math.floor(progress)}%` : task.status === "Completed" ? "100%" : `${task.total_bytes ? `${Math.floor(percent)}%` : ""}${isRunning ? `${task.total_bytes ? " · " : ""}${formatBytes(displayedSpeeds[task.id] ?? 0)}/s` : ""}`}</span>
            <div className="task-actions">
              <button className="icon-button" title="打开目录" aria-label={`打开 ${task.file.name} 所在目录`} onClick={() => onOpen(task.save_dir)}><FolderOpen size={15} /></button>
              {task.status !== "Completed" && <button className="icon-button" disabled={pending.has(task.id)} title={isRunning ? "暂停" : "继续下载"} aria-label={`${isRunning ? "暂停" : "继续下载"} ${task.file.name}`} onClick={() => void onAction(isRunning ? "pause" : "start", [task.id])}>{isRunning ? <Pause size={15} /> : <Play size={15} />}</button>}
              <button className="icon-button" disabled={isRunning || pending.has(task.id)} title="移除记录，保留本地文件与续传数据" aria-label={`移除 ${task.file.name} 的任务记录`} onClick={() => void onAction("remove", [task.id])}><Trash2 size={15} /></button>
            </div>
          </div>
          {task.error && <p className="task-error" role={task.status === "Failed" ? "alert" : undefined}>{task.error}</p>}
        </article>;
      })}
    </div>
  </section>;
}
