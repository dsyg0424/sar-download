import { expect, test } from "bun:test";
import { beamModeOptions, reconcileOptions, optionsFor, validateSearchOptions, validSearchDate } from "../src/features/sar-download/model/searchOptions";
import type { SarSearchRequest } from "../src/features/sar-download/services/sarSearchApi";
const request: SarSearchRequest = { bounds: [114, 30, 115, 31], start: "2024-02-29", end: "2024-03-01", product: "SLC", direction: "", polarization: "" };
test("latest type or satellite wins and incompatible S1 filters reset", () => {
  const old = { ...request, imageType: "SLC", platform: "Sentinel-1A", beamMode: "IW", polarization: "VV+VH" };
  const nisar = reconcileOptions(old, { imageType: "GSLC" });
  expect(nisar.request.platform).toBe("");
  expect(nisar.request.beamMode).toBe("");
  expect(nisar.request.polarization).toBe("");
  expect(nisar.adjustment).not.toBe("");
  const sentinel = reconcileOptions(nisar.request, { platform: "Sentinel-1B" });
  expect(sentinel.request.imageType).toBe("");
  expect(sentinel.request.platform).toBe("Sentinel-1B");
  const burst = reconcileOptions({ ...old, beamMode: "SM" }, { imageType: "BURST" });
  expect(burst.request.beamMode).toBe("");
  expect(burst.request.polarization).toBe("");
  expect(optionsFor(burst.request).beams.map(o => o.value)).toEqual(["", "IW", "EW"]);
  expect(optionsFor(burst.request).polarizations.map(o => o.value)).toEqual(["", "VV", "VH", "HH", "HV"]);
  expect(reconcileOptions(old, { beamMode: "WV" }).request.polarization).toBe("");
  expect(reconcileOptions(old, { platform: "Sentinel-1B" }).request.polarization).toBe("VV+VH");
});
test("multi-product selection controls satellites and compatible polarization", () => {
  const initial = { ...request, productTypes: ["SLC"], satellites: ["Sentinel-1A", "Sentinel-1C"], polarization: "VV+VH" };
  const mixed = reconcileOptions(initial, { productTypes: ["SLC", "BURST", "GSLC"] }).request;
  expect(mixed.satellites).toEqual(initial.satellites);
  expect(mixed.polarization).toBe("");
  expect(optionsFor(mixed).polarizations.map(o => o.value)).toEqual(["", "VV", "HH"]);
  const nisar = reconcileOptions(initial, { productTypes: ["GSLC", "RSLC"] }).request;
  expect(nisar.satellites).toEqual([]);
  expect(nisar.polarization).toBe("");
  expect(() => validateSearchOptions(nisar)).not.toThrow();
});
test("NISAR accepts only complex products without Sentinel beam filters", () => {
  const nisar = { ...request, platform: "NISAR", product: "GSLC,RSLC" };
  expect(() => validateSearchOptions(nisar)).not.toThrow();
  expect(() => validateSearchOptions({ ...nisar, product: "GCOV" })).toThrow();
  expect(() => validateSearchOptions({ ...nisar, beamMode: "IW" })).toThrow();
});

test("dates reject impossible calendar values and reversed ranges", () => {
  expect(() => validateSearchOptions({ ...request, imageType: "BURST", platform: "NISAR" })).toThrow();
  expect(() => validateSearchOptions({ ...request, imageType: "GSLC", platform: "Sentinel-1A" })).toThrow();
  expect(() => validateSearchOptions({ ...request, imageType: "GSLC", platform: "" })).not.toThrow();
  expect(validSearchDate("2024-02-29")).toBe(true);
  for (const date of ["", "2025-02-29", "2024-04-31", "2024-13-01"]) expect(validSearchDate(date)).toBe(false);
  expect(() => validateSearchOptions({ ...request, end: "2024-02-01" })).toThrow();
});
test("all beam choices are valid but unrelated platforms and polarizations are rejected", () => {
  for (const option of beamModeOptions) expect(() => validateSearchOptions({ ...request, beamMode: option.value })).not.toThrow();
  expect(() => validateSearchOptions({ ...request, platform: "Sentinel-1D", polarization: "VV+VH" })).not.toThrow();
  expect(() => validateSearchOptions({ ...request, platform: "ALOS" })).toThrow();
  expect(() => validateSearchOptions({ ...request, polarization: "VH" })).toThrow();
  expect(() => validateSearchOptions({ ...request, direction: "UP" })).toThrow();
});
