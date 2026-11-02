import { create } from "zustand";
import {
  captureSearch,
  mergeResults,
  validateResponse,
  type SearchBackup,
  type ProductProgress,
} from "../model/searchProgress";
import { pathKey } from "../model/groups";
import {
  reconcileOptions,
  validateSearchOptions,
} from "../model/searchOptions";
import {
  searchSar,
  cancelSar,
  type SarProduct,
  type SarSearchRequest,
} from "../services/sarSearchApi";

export function createSarSearchStore(
  runSearch: typeof searchSar = (request, onProgress, requestId) =>
    searchSar(request, onProgress, requestId),
  cancelSearch: typeof cancelSar = cancelSar,
) {
  return create<{
    request: SarSearchRequest;
    resultRequest: SarSearchRequest | null;
    adjustment: string;
    areaSelected: boolean;
    results: SarProduct[];
    selected: string[]; // Download selection: individual SarProduct.id values.
    activeGroup: string | null;
    activeScene: string | null;
    activateScene: (id: string, locate?: boolean) => void;
    clearActiveScene: () => void;
    activation: number;
    focusRequest: { group: string; scene?: string; token: number } | null;
    activate: (group: string, locate?: boolean) => void;
    busy: boolean;
    cancelling: boolean;
    requestId: string | null;
    cancel: () => Promise<void>;
    searchBackup: SearchBackup | null;
    searchProgress: Record<string, ProductProgress>;
    error: string;
    searched: boolean;
    warnings: string[];
    limitReached: boolean;
    update: (patch: Partial<SarSearchRequest>) => void;
    toggle: (id: string) => void;
    search: () => Promise<void>;
    reset: () => void;
    resetVersion: number;
  }>((set, get) => ({
    adjustment: "",
    resultRequest: null,
    request: {
      bounds: [],
      beamMode: "",
      platform: "",
      start: "",
      end: "",
      productTypes: [],
      satellites: [],
      direction: "",
      polarization: "",
    },
    results: [],
    resetVersion: 0,
    reset: () => {
      if (get().busy) return;
      set({
        resetVersion: get().resetVersion + 1,
        searchBackup: null,
        searchProgress: {},
        adjustment: "",
        request: {
          bounds: [],
          beamMode: "",
          platform: "",
          start: "",
          end: "",
          productTypes: [],
          satellites: [],
          direction: "",
          polarization: "",
        },
        results: [],
        resultRequest: null,
        selected: [],
        areaSelected: false,
        activeGroup: null,
        activeScene: null,
        focusRequest: null,
        error: "",
        searched: false,
        warnings: [],
        limitReached: false,
      });
    },
    areaSelected: false,
    selected: [],
    activeGroup: null,
    activeScene: null,
    activateScene: (id, locate = false) => {
      const product = get().results.find((item) => item.id === id);
      if (!product) return;
      const group = pathKey(product);
      set({
        activeScene: id,
        activeGroup: group,
        activation: get().activation + 1,
        ...(locate
          ? {
              focusRequest: {
                group,
                scene: id,
                token: (get().focusRequest?.token ?? 0) + 1,
              },
            }
          : {}),
      });
    },
    clearActiveScene: () => {
      if (!get().activeScene && !get().activeGroup && !get().focusRequest) return;
      set({ activeScene: null, activeGroup: null, focusRequest: null, activation: get().activation + 1 });
    },
    activation: 0,
    focusRequest: null,
    activate: (group, locate = false) =>
      set({
        activeGroup: group,
        activeScene: null,
        activation: get().activation + 1,
        ...(locate
          ? {
              focusRequest: {
                group,
                token: (get().focusRequest?.token ?? 0) + 1,
              },
            }
          : {}),
      }),
    searchBackup: null,
    searchProgress: {},
    busy: false,
    cancelling: false,
    requestId: null,
    cancel: async () => {
      const id = get().requestId;
      if (!get().busy || get().cancelling || !id) return;
      set({ cancelling: true });
      try { await cancelSearch(id); }
      catch (error) { if (get().requestId === id) set({ cancelling: false, error: `取消失败：${String(error)}` }); }
    },
    error: "",
    searched: false,
    warnings: [],
    limitReached: false,
    update: (patch) => {
      if (get().busy) return;
      set(reconcileOptions(get().request, patch));
    },
    toggle: (id) =>
      set({
        selected: get().selected.includes(id)
          ? get().selected.filter((item) => item !== id)
          : [...get().selected, id],
      }),
    search: async () => {
      if (get().busy) return;
      const requestId = crypto.randomUUID();
      const resetVersion = get().resetVersion;
      const current = () =>
        resetVersion === get().resetVersion && requestId === get().requestId;
      const backup = captureSearch(get());
      let accepting = true;
      let previewed = false;
      let previewError: unknown;
      set({ busy: true, cancelling: false, requestId, error: "", searchBackup: backup, searchProgress: {} });
      try {
        const request = structuredClone({
          ...get().request,
        });
        const [west, south, east, north] = request.bounds;
        if (
          !request.bounds.every(Number.isFinite) ||
          request.bounds.length !== 4 ||
          !(
            west >= -180 &&
            west < east &&
            east <= 180 &&
            south >= -90 &&
            south < north &&
            north <= 90
          )
        )
          throw new Error(
            "范围无效：请分别填写西、东经度（±180）和南、北纬度（±90），保证西小于东、南小于北。",
          );
        validateSearchOptions(request);
        const response = await runSearch(request, (event) => {
          if (!accepting || !current() || !get().busy || get().cancelling) return;
          try {
            const previous = get().searchProgress[event.product];
            if (previous?.complete) return;
            const progress = {
              complete: false,
              stage: previous?.stage ?? "",
              count: previous?.count ?? 0,
            };
            if (event.event === "stage" && event.state === "started") {
              progress.stage = event.stage;
            }
            if (event.event === "product_complete" || event.event === "product_page") {
              validateResponse(event.data);
              const first = !previewed;
              previewed = true;
              progress.complete = event.event === "product_complete";
              const before = first ? [] : get().results;
              const results = mergeResults(
                event.event === "product_complete" ? before.filter(item => item.product !== event.product) : before,
                event.data.results,
              );
              progress.count = results.filter(item => item.product === event.product).length;
              set({
                results,
                resultRequest: request,
                searched: true,
                limitReached: false,
                warnings: [],
                ...(first
                  ? {
                      selected: [],
                      activeGroup: null,
                      activeScene: null,
                      focusRequest: null,
                    }
                  : {}),
              });
            }
            set({
              searchProgress: {
                ...get().searchProgress,
                [event.product]: progress,
              },
            });
          } catch (error) {
            previewError = error;
          }
        }, requestId);
        if (!current()) return;
        if (get().cancelling) throw new Error("SAR_SEARCH_CANCELLED");
        if (previewError) throw previewError;
        validateResponse(response);
        set({
          ...response,
          error: "",
          resultRequest: request,
          warnings: response.warnings ?? [],
          searched: true,
          selected: previewed
            ? get().selected.filter((id) =>
                response.results.some((item) => item.id === id),
              )
            : [],
          ...(!previewed
            ? { activeGroup: null, activeScene: null, focusRequest: null }
            : {}),
        });
      } catch (error) {
        if (current()) {
          if (!get().cancelling && !String(error).includes("SAR_SEARCH_CANCELLED")) console.warn("影像检索失败", error);
          set({
            ...backup,
            error: get().cancelling || String(error).includes("SAR_SEARCH_CANCELLED") ? "检索已取消，已恢复上次结果。" : String(error),
            activation: get().activation + 1,
          });
        }
      } finally {
        accepting = false;
        if (current())
          set({ busy: false, cancelling: false, requestId: null, searchBackup: null, searchProgress: {} });
      }
    },
  }));
}
export const useSarSearchStore = createSarSearchStore();
