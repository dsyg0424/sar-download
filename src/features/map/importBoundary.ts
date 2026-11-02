import type { Region } from "./region";
export const boundaryAccept = ".geojson,.json,.kml,.kmz,.zip,.shp,.prj,.dbf,.shx,.cpg";

export function importBoundary(files: File[]): Promise<Region> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./boundary.worker.ts", import.meta.url), { type: "module" });
    const cleanup = () => { clearTimeout(timeout); worker.terminate(); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error("边界解析超时，请简化边界后重试")); }, 60000);
    worker.onmessage = (event: MessageEvent<{ geometry?: Region; error?: string }>) => {
      cleanup();
      if (event.data.geometry) resolve(event.data.geometry);
      else reject(new Error(event.data.error || "边界文件解析失败"));
    };
    worker.onerror = () => { cleanup(); reject(new Error("边界解析失败，请检查文件格式与投影定义")); };
    try { worker.postMessage(files); }
    catch (error) { cleanup(); reject(error); }
  });
}
