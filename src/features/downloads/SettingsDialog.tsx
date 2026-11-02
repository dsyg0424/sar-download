import { useEffect, useRef, useState } from "react";
import { Eye, EyeOff, FolderOpen, Loader2, X } from "lucide-react";
import { downloadApi, transferLimits, transferSettingsError, type Settings } from "./api";

export function SettingsDialog({ settings, loading = false, onSave, onClose }: {
  settings: Settings; loading?: boolean; onSave: (settings: Settings) => Promise<void>; onClose: () => void;
}) {
  const [draft, setDraft] = useState(settings);
  const [saving, setSaving] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [visible, setVisible] = useState(false);
  const [error, setError] = useState("");
  const dirty = useRef(false);
  const dialog = useRef<HTMLDialogElement>(null);
  const busy = saving || choosing;
  useEffect(() => { dialog.current?.showModal(); }, []);
  useEffect(() => { if (!dirty.current) setDraft(settings); }, [settings]);
  const update = (patch: Partial<Settings>) => { dirty.current = true; setDraft((s) => ({ ...s, ...patch })); };
  return <dialog ref={dialog} className="settings-dialog" aria-labelledby="download-settings-heading" onCancel={(e) => { if (busy) e.preventDefault(); else onClose(); }}>
    <form onSubmit={async (event) => {
      event.preventDefault(); if (busy || loading) return;
      const rangeError = transferSettingsError(draft);
      if (rangeError) { setError(rangeError); return; }
      const username = draft.username.trim();
      if (Boolean(username) !== Boolean(draft.password)) { setError("请同时填写用户名和密码，或清空两项后仅保存其他设置。"); return; }
      setSaving(true); setError("");
      try { await onSave({ ...draft, username, rememberPassword: Boolean(username && draft.password), credentialWarning: null }); onClose(); }
      catch (error) { setError(String(error)); } finally { setSaving(false); }
    }}>
      <header className="dialog-heading"><h2 id="download-settings-heading">下载设置</h2>
        <button type="button" className="icon-button" aria-label="关闭设置" disabled={busy} onClick={onClose}><X size={18} /></button></header>
      <div className="settings-content" aria-busy={loading}>
        {loading && <p className="settings-loading" role="status"><Loader2 size={14} className="spinner" />正在读取已保存设置…</p>}
        <fieldset disabled={busy || loading} className="settings-section settings-storage">
          <legend>文件保存</legend>
          <label htmlFor="download-output-dir">保存目录</label>
          <div className="input-action"><input id="download-output-dir" required value={draft.outputDir} placeholder="选择保存影像的文件夹" onChange={(e) => update({ outputDir: e.target.value })} />
            <button type="button" aria-label="选择下载目录" title="选择文件夹" onClick={async () => {
              setChoosing(true); setError("");
              try { const path = await downloadApi.chooseDirectory(); if (path) update({ outputDir: path }); }
              catch (error) { setError(String(error)); } finally { setChoosing(false); }
            }}><FolderOpen size={16} /></button></div>
          <label className="check-label"><input type="checkbox" checked={draft.organize} onChange={(e) => update({ organize: e.target.checked })} />按卫星和产品分类保存</label>
        </fieldset>
        <fieldset disabled={busy || loading} className="settings-section settings-account">
          <legend>Earthdata 账号</legend>
          <label htmlFor="earthdata-username">用户名</label>
          <input id="earthdata-username" aria-label="Earthdata 用户名" autoComplete="username" value={draft.username} onChange={(e) => update({ username: e.target.value })} />
          <label htmlFor="earthdata-password">密码</label>
          <div className="password-input"><input id="earthdata-password" aria-label="Earthdata 密码" type={visible ? "text" : "password"} autoComplete="current-password" aria-describedby="password-storage-note" value={draft.password} onChange={(e) => update({ password: e.target.value })} />
            <button type="button" className="icon-button" aria-label={visible ? "隐藏密码" : "显示密码"} aria-pressed={visible} onClick={() => setVisible((value) => !value)}>{visible ? <EyeOff size={15} /> : <Eye size={15} />}</button></div>
          <div className="account-storage"><p id="password-storage-note" className="muted">保存后自动恢复账号密码；密码由系统凭据管理器保存。</p>
            <button type="button" disabled={!draft.username && !draft.password} className="settings-clear-account" onClick={() => update({ username: "", password: "", rememberPassword: false })}>清空账号</button></div>
        </fieldset>
        <fieldset disabled={busy || loading} className="settings-section settings-transfer" aria-describedby="concurrency-note">
          <legend>传输</legend>
          <div className="settings-grid">
            <label>同时下载数<input aria-label="同时下载数" type="number" min={transferLimits.parallelFiles.min} max={transferLimits.parallelFiles.max} step="1" required value={draft.parallelFiles} onChange={(e) => update({ parallelFiles: Number(e.target.value) })} /></label>
            <label>单文件连接数<input aria-label="单文件连接数" type="number" min={transferLimits.concurrency.min} max={transferLimits.concurrency.max} step="1" required value={draft.concurrency} onChange={(e) => update({ concurrency: Number(e.target.value) })} /></label>
            <label className="settings-speed">限速（MiB/s）<input type="number" min="0" max="10000" step="1" required aria-describedby="speed-limit-note" value={draft.speedLimitMib} onChange={(e) => update({ speedLimitMib: Number(e.target.value) })} /><span id="speed-limit-note" className="muted">0 表示不限速</span></label>
          </div>
          <span id="concurrency-note" className="sr-only">新任务使用当前并发设置。</span>
        </fieldset>
        {error && <p className="error-message settings-error" role="alert">{error}</p>}
      </div>
      <footer><button type="button" className="btn btn-secondary" disabled={busy} onClick={onClose}>取消</button><button className="btn btn-primary" disabled={busy || loading}>{saving ? <><Loader2 size={14} className="spinner" />保存中…</> : "保存设置"}</button></footer>
    </form>
  </dialog>;
}
