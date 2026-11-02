import type { SarSearchRequest } from "../services/sarSearchApi";
import { localDate } from "../../../shared/utils/localDate";

export const beamModeOptions = [
  { value: "", label: "不限" },
  { value: "IW", label: "IW · 干涉宽幅" },
  { value: "EW", label: "EW · 超宽幅" },
  { value: "SM", label: "SM · 全部条带" },
  ...[1, 2, 3, 4, 5, 6].map(n => ({ value: `S${n}`, label: `S${n} · 条带 ${n}` })),
  { value: "WV", label: "WV · 波模式" },
];
export const satelliteOptions = [
  { value: "", label: "不限" },
  ...["A", "B", "C", "D"].map(n => ({ value: `Sentinel-1${n}`, label: `Sentinel-1${n}` })),
  { value: "NISAR", label: "NISAR" },
];
export const imageTypeOptions = [
  { value: "", label: "不限" },
  { value: "GSLC", label: "NISAR GSLC" },
  { value: "RSLC", label: "NISAR RSLC" },
  { value: "SLC", label: "S1 SLC" },
  { value: "BURST", label: "S1 Bursts" },
];
export const nisarOnly = (request: SarSearchRequest) => request.productTypes !== undefined
  ? request.productTypes.length > 0 && request.productTypes.every(t => ["GSLC", "RSLC"].includes(t))
  : request.platform === "NISAR" || ["GSLC", "RSLC"].includes(request.imageType ?? "");
export const directionOptions = [
  { value: "", label: "不限" },
  { value: "ASCENDING", label: "升轨 · 南向北" },
  { value: "DESCENDING", label: "降轨 · 北向南" },
];
export const polarizationOptions = [
  { value: "", label: "不限" },
  { value: "VV+VH", label: "VV+VH · 双极化" },
  { value: "HH+HV", label: "HH+HV · 双极化" },
  { value: "VV", label: "VV · 单极化" },
  { value: "HH", label: "HH · 单极化" },
];
export const validSearchDate = (value: string) => {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const parsed = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(parsed.getTime()) && parsed.toISOString().slice(0, 10) === value;
};
export function sarQueryKey(request: SarSearchRequest) {
  return JSON.stringify([request.bounds, request.geometry ?? null, request.start, request.end,
    [...(request.productTypes ?? [])].sort(), [...(request.satellites ?? [])].sort(), request.direction, request.polarization]);
}
export function optionsFor(request: SarSearchRequest) {
  if (request.productTypes !== undefined) {
    const types = request.productTypes.length ? request.productTypes : ["GSLC", "RSLC", "SLC", "BURST"];
    const values = nisarOnly(request) ? [""] : types.includes("BURST")
      ? types.includes("SLC") ? ["", "VV", "HH"] : ["", "VV", "VH", "HH", "HV"]
      : ["", "VV+VH", "HH+HV", "VV", "HH"];
    return { beams: beamModeOptions.slice(0, 1), polarizations: values.map(value => ({ value, label: value || "不限" })) };
  }
  const burst = request.imageType === "BURST";
  return {
    beams: nisarOnly(request) ? beamModeOptions.slice(0, 1) : burst ? beamModeOptions.filter(o => ["", "IW", "EW"].includes(o.value)) : beamModeOptions,
    polarizations: nisarOnly(request) ? polarizationOptions.slice(0, 1) : burst
      ? ["", "VV", "VH", "HH", "HV"].map(value => ({ value, label: value || "不限" }))
      : request.beamMode === "WV" ? polarizationOptions.filter(o => ["", "VV", "HH"].includes(o.value)) : polarizationOptions,
  };
}
const conflict = (r: SarSearchRequest) => (r.platform === "NISAR" && ["SLC", "BURST"].includes(r.imageType ?? "")) ||
  (r.platform?.startsWith("Sentinel-1") && ["GSLC", "RSLC"].includes(r.imageType ?? ""));
export function reconcileOptions(previous: SarSearchRequest, patch: Partial<SarSearchRequest>) {
  const request = { ...previous, ...patch };
  if (request.productTypes !== undefined) {
    request.beamMode = "";
    if (nisarOnly(request)) { request.satellites = []; }
    if (!optionsFor(request).polarizations.some(o => o.value === request.polarization)) request.polarization = "";
    return { request, adjustment: "" };
  }
  const cleared: string[] = [];
  if (conflict(request)) {
    if (patch.imageType !== undefined) { request.platform = ""; cleared.push("卫星"); }
    else { request.imageType = ""; cleared.push("影像类型"); }
  }
  const options = optionsFor(request);
  if (!options.beams.some(o => o.value === (request.beamMode ?? ""))) { request.beamMode = ""; cleared.push("S1成像方式"); }
  if (!optionsFor(request).polarizations.some(o => o.value === request.polarization)) { request.polarization = ""; cleared.push("S1极化方式"); }
  return { request, adjustment: cleared.length ? `已将不兼容的${cleared.join("、")}重置为不限。` : "" };
}
export function validateSearchOptions(request: SarSearchRequest) {
  if (request.productTypes !== undefined) {
    if (!Array.isArray(request.productTypes) || request.productTypes.some(t => !["GSLC", "RSLC", "SLC", "BURST"].includes(t))) throw new Error("产品类型无效");
    if (!Array.isArray(request.satellites) || request.satellites.some(s => !["Sentinel-1A", "Sentinel-1B", "Sentinel-1C", "Sentinel-1D"].includes(s))) throw new Error("S1卫星无效");
    if (nisarOnly(request) && request.satellites.length) throw new Error("NISAR 产品不适用 S1 卫星条件");
  }
  if (request.imageType !== undefined) {
    if (!imageTypeOptions.some(o => o.value === request.imageType)) throw new Error("影像类型无效");
    if ((request.platform === "NISAR" && ["SLC", "BURST"].includes(request.imageType)) ||
        (request.platform?.startsWith("Sentinel-1") && ["GSLC", "RSLC"].includes(request.imageType)))
      throw new Error("卫星与影像类型不匹配，请调整筛选条件。");
  }
  if (request.start > localDate() || request.end > localDate())
    throw new Error("筛选日期不能晚于本地今天。");
  if (request.imageType === undefined && request.platform === "NISAR" && (request.beamMode || request.polarization || request.product !== "GSLC,RSLC"))
    throw new Error("NISAR 仅检索 GSLC、RSLC，请清除 Sentinel-1 专用条件。");
  if (!validSearchDate(request.start) || !validSearchDate(request.end) || request.start < "2014-01-01" || request.end < "2014-01-01" || request.start > request.end)
    throw new Error("请填写有效的开始和结束日期，开始日期不能晚于结束日期。");
  for (const [value, options, name] of [
    [request.beamMode ?? "", optionsFor(request).beams, "成像方式"],
    [request.platform ?? "", satelliteOptions, "卫星"],
    [request.direction, directionOptions, "方向"],
    [request.polarization, optionsFor(request).polarizations, "极化"],
  ] as const) {
    if (!options.some(option => option.value === value)) throw new Error(`${name}选项无效，请重新选择。`);
  }
}
