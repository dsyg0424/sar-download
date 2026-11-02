import { invoke } from "@tauri-apps/api/core";
import type { SarProduct } from "../sar-download/services/sarSearchApi";

export const transferLimits = { parallelFiles: { min: 1, max: 32 }, concurrency: { min: 1, max: 64 } } as const;
export function transferSettingsError(settings: Pick<Settings, "parallelFiles" | "concurrency">): string {
  for (const [key, label] of [["parallelFiles", "同时下载数"], ["concurrency", "单文件连接数"]] as const) {
    const { min, max } = transferLimits[key];
    if (!Number.isInteger(settings[key]) || settings[key] < min || settings[key] > max) return `${label}应为 ${min}–${max} 的整数`;
  }
  return "";
}

export interface Settings {
  outputDir: string;
  username: string;
  password: string;
  rememberPassword: boolean;
  credentialWarning?: string | null;
  concurrency: number;
  parallelFiles: number;
  speedLimitMib: number;
  organize: boolean;
}
export interface DownloadTask {
  id: string;
  file: { name: string; url: string; size: number };
  companion?: { name: string; url: string; size: number } | null;
  active_file?: string | null;
  product: string;
  save_dir: string;
  status: string;
  downloaded_bytes: number;
  total_bytes: number;
  speed_bps: number;
  verified_bytes: number;
  error: string | null;
  revision: number;
}
export interface CleanupPlan { token: string; items: {id:string;path:string;bytes:number;category:string}[]; totalBytes:number }
export interface CleanupResult { removedFiles:number;freedBytes:number;errors:string[] }
export interface CleanupActions {
  preview:()=>Promise<CleanupPlan>;
  execute:(token:string,ids:string[])=>Promise<CleanupResult>;
}
export const defaults: Settings = {
  outputDir: "", username: "", password: "", rememberPassword: false,
  concurrency: 8, parallelFiles: 3, speedLimitMib: 0, organize: true,
};
export const running = (status: string) => ["Queued", "Connecting", "Downloading", "Verifying", "Publishing"].includes(status);
export const statusLabels: Record<string, string> = {
  Ready: "待下载", Queued: "排队中", Connecting: "连接中", Downloading: "下载中",
  Paused: "已暂停", Verifying: "校验中", Publishing: "保存中", Completed: "已完成", Failed: "失败",
};
export function mergeTask(current: DownloadTask[], task: DownloadTask) {
  const old = current.find((t) => t.id === task.id);
  if (old && old.revision > task.revision) return current;
  return old ? current.map((t) => t.id === task.id ? task : t) : [...current, task];
}
export function mergeSnapshot(current: DownloadTask[], incoming: DownloadTask[], protectedIds = new Set<string>()) {
  const oldById = new Map(current.map(task => [task.id, task]));
  const incomingIds = new Set(incoming.map(task => task.id));
  return [...incoming.map(task => {
    const old = oldById.get(task.id);
    return old && old.revision > task.revision ? old : task;
  }), ...current.filter(task => protectedIds.has(task.id) && !incomingIds.has(task.id))];
}
export function formatBytes(value: number) {
  if (value <= 0 || !Number.isFinite(value)) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const unit = Math.min(Math.floor(Math.log(value) / Math.log(1024)), 4);
  return `${(value / 1024 ** unit).toFixed(unit > 1 ? 2 : 0)} ${units[unit]}`;
}
export const downloadApi = {
  previewCleanup: (outputDir:string) => invoke<CleanupPlan>("preview_download_cleanup", {outputDir}),
  executeCleanup: (token:string,ids:string[]) => invoke<CleanupResult>("execute_download_cleanup", {token,ids}),
  loadSettings: () => invoke<Settings>("load_settings"),
  saveSettings: ({ password, credentialWarning: _warning, ...settings }: Settings) => invoke<Settings>("save_settings", { settings, password }),
  list: () => invoke<DownloadTask[]>("list_downloads"),
  add: (products: SarProduct[], { password: _password, credentialWarning: _warning, ...settings }: Settings) =>
    invoke<{ added: string[]; skipped: number }>("add_downloads", { products, settings }),
  start: (id: string, { password, credentialWarning: _warning, ...settings }: Settings) =>
    invoke("start_download", { id, username: settings.username, password, settings }),
  pause: (id: string) => invoke("pause_download", { id }),
  removeMany: (ids: string[]) => invoke<void>("remove_downloads", { ids }),
  remove: (id: string) => invoke("remove_download", { id }),
  chooseDirectory: () => invoke<string | null>("choose_output_dir"),
  openDirectory: (path: string) => invoke("open_output_dir", { path }),
};
