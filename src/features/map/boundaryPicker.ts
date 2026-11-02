import { invoke } from "@tauri-apps/api/core";

export function decodeBoundarySelection(buffer: ArrayBuffer): File[] | null {
  if (!buffer.byteLength) return null;
  const bytes = new Uint8Array(buffer);
  const invalid = () => new Error("边界文件读取结果不完整，请重新选择");
  if (bytes.length < 4) throw invalid();
  const headerSize = new DataView(buffer).getUint32(0, true);
  if (headerSize > bytes.length - 4) throw invalid();
  const parts: unknown = JSON.parse(new TextDecoder().decode(bytes.subarray(4, 4 + headerSize)));
  if (!Array.isArray(parts) || !parts.length || parts.length > 2) throw invalid();
  let offset = 4 + headerSize;
  const files = parts.map((part: { name?: unknown; size?: unknown }) => {
    if (!part || typeof part.name !== "string" || typeof part.size !== "number" || !Number.isSafeInteger(part.size) || part.size < 0 || part.size > bytes.length - offset) throw invalid();
    const file = new File([buffer.slice(offset, offset + part.size)], part.name);
    offset += part.size;
    return file;
  });
  if (offset !== bytes.length) throw invalid();
  return files;
}

export async function chooseBoundaryFiles(): Promise<File[] | null> {
  return decodeBoundarySelection(await invoke<ArrayBuffer>("choose_boundary_file"));
}
