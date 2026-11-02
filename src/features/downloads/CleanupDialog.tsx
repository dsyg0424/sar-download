import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { formatBytes, type CleanupActions, type CleanupPlan, type CleanupResult } from "./api";

export function CleanupDialog({actions,onClose}:{actions:CleanupActions;onClose:()=>void}) {
  const dialog=useRef<HTMLDialogElement>(null);
  const [plan,setPlan]=useState<CleanupPlan|null>(null);
  const [selected,setSelected]=useState<string[]>([]);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  const [result,setResult]=useState<CleanupResult|null>(null);
  const generation=useRef(0);
  const scan=async()=>{
    const token=++generation.current;setBusy(true);setError("");setPlan(null);setResult(null);setSelected([]);
    try {const next=await actions.preview();if(token!==generation.current)return;setPlan(next);setSelected(next.items.map(item=>item.id));}
    catch(error){if(token===generation.current)setError(String(error));}
    finally{if(token===generation.current)setBusy(false);}
  };
  useEffect(()=>{dialog.current?.showModal();void scan();return()=>{generation.current++;};},[]);
  const bytes=plan?.items.filter(item=>selected.includes(item.id)).reduce((sum,item)=>sum+item.bytes,0)??0;
  return createPortal(<dialog ref={dialog} className="settings-dialog cleanup-dialog" aria-labelledby="cleanup-heading"
    onCancel={event=>{event.preventDefault();if(!busy)onClose();}}
    onKeyDown={event=>event.stopPropagation()}>
    <div className="dialog-heading"><h2 id="cleanup-heading">空间清理</h2><button type="button" className="icon-button" disabled={busy} aria-label="关闭空间清理" onClick={onClose}><X size={16}/></button></div>
    <div className="cleanup-content">
      <p>清理已移除任务遗留的临时文件和续传记录。已下载的数据、校验记录及仍被任务引用的文件会保留。可单独清理临时文件；续传记录需与对应任务残留一并清理。</p>
      {busy&&<p role="status">{plan?"正在清理…":"正在扫描…"}</p>}
      {error&&<p role="alert" className="error-message">{error}</p>}
      {result&&<p role="status">已清理 {result.removedFiles} 个文件，释放 {formatBytes(result.freedBytes)}。{result.errors.length>0&&` ${result.errors.length} 个文件清理失败。`}</p>}
      {result?.errors.map((error,index)=><p key={index} role="alert" className="error-message">{error}</p>)}
      {plan&&!result&&<>
        {plan.items.length>0?<>
          <label className="cleanup-select-all"><input type="checkbox" aria-label="选择全部可清理文件" checked={selected.length===plan.items.length} disabled={busy} onChange={event=>setSelected(event.target.checked?plan.items.map(item=>item.id):[])}/>全选 <span>{formatBytes(bytes)} / {plan.items.length} 个文件</span></label>
          <ul className="cleanup-list">{plan.items.map(item=><li key={item.id}>
            <input type="checkbox" aria-label={`清理 ${item.path}`} disabled={busy} checked={selected.includes(item.id)} onChange={event=>setSelected(old=>event.target.checked?[...old,item.id]:old.filter(id=>id!==item.id))}/>
            <div><strong title={item.path}>{item.path}</strong><span>{item.category}</span></div><span>{formatBytes(item.bytes)}</span>
          </li>)}</ul>
        </>:<p role="status">没有可清理的下载临时文件。</p>}
      </>}
    </div>
    <footer><button className="btn btn-secondary" disabled={busy} onClick={()=>void scan()}>重新扫描</button><button className="btn btn-secondary" disabled={busy} onClick={onClose}>关闭</button>
      {!result&&<button className="btn btn-primary" disabled={busy||!plan||!selected.length} onClick={async()=>{
        if(!plan)return;setBusy(true);setError("");
        try{setResult(await actions.execute(plan.token,selected));}
        catch(error){setError(String(error));setPlan(null);setSelected([]);}
        finally{setBusy(false);}
      }}>清理所选（{selected.length}）</button>}
    </footer>
  </dialog>,document.body);
}
