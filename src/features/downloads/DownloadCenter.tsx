import { useEffect, useId, useRef, useState } from "react";
import { ArrowDownToLine, ChevronDown } from "lucide-react";
import { DownloadPanel } from "./DownloadPanel";
import { running, type DownloadTask, type CleanupActions } from "./api";

export function DownloadCenter({ tasks, pending, onAction, onOpen, revealVersion = 0, cleanup }: {
  tasks: DownloadTask[];
  pending: Set<string>;
  onAction: (action: "start" | "pause" | "remove", ids: string[]) => Promise<void>;
  onOpen: (path: string) => void;
  revealVersion?: number;
  cleanup?: CleanupActions;
}) {
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const active = tasks.some((task) => running(task.status));
  useEffect(() => { if (revealVersion > 0) setOpen(true); }, [revealVersion]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!container.current?.contains(event.target as Node)) setOpen(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); setOpen(false); trigger.current?.focus(); } };
    document.addEventListener("pointerdown", outside); document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside); document.removeEventListener("keydown", escape); };
  }, [open]);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  return <section ref={container} className={`download-center${open ? " is-open" : ""}`} aria-label="下载中心"
    onKeyDown={(event) => {
      if (event.key === "Escape" && open) { event.preventDefault(); event.stopPropagation(); close(); }
    }}>
    <button ref={trigger} type="button" className="download-launcher" aria-expanded={open} aria-controls={id}
      onClick={() => setOpen((value) => !value)}>
      {open ? <ChevronDown size={15} /> : <ArrowDownToLine size={15} />}
      <span>下载</span><span className="download-count">({tasks.length})</span>
      {active && <span className="transfer-indicator" aria-label="有任务正在下载" />}
    </button>
    <div id={id} className="download-drawer glass-panel" inert={!open} aria-hidden={!open}>
      <DownloadPanel visible={open} tasks={tasks} pending={pending} onAction={onAction} onOpen={onOpen} onClose={close} cleanup={cleanup} />
    </div>
  </section>;
}
