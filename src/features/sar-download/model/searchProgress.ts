import type {
  SarProduct,
  SarSearchRequest,
  SarSearchResponse,
} from "../services/sarSearchApi";

export interface SearchBackup {
  results: SarProduct[];
  resultRequest: SarSearchRequest | null;
  selected: string[];
  activeGroup: string | null;
  activeScene: string | null;
  focusRequest: { group: string; scene?: string; token: number } | null;
  searched: boolean;
  warnings: string[];
  limitReached: boolean;
}
export interface ProductProgress {
  complete: boolean;
  stage: string;
  count: number;
}

export function captureSearch(state: SearchBackup): SearchBackup {
  const {
    results,
    resultRequest,
    selected,
    activeGroup,
    activeScene,
    focusRequest,
    searched,
    warnings,
    limitReached,
  } = state;
  return {
    results,
    resultRequest,
    selected,
    activeGroup,
    activeScene,
    focusRequest,
    searched,
    warnings,
    limitReached,
  };
}

export function validateResponse(response: SarSearchResponse) {
  if (
    response.limitReached ||
    response.results.some(
      (item) =>
        !item.geometry ||
        !["Polygon", "MultiPolygon"].includes(item.geometry.type),
    )
  ) {
    throw new Error("服务器返回的影像结果不完整或边界无效，请重试");
  }
}

export function mergeResults(before: SarProduct[], next: SarProduct[]) {
  const items = new Map(before.map((item) => [item.id, item]));
  for (const item of next) items.set(item.id, item);
  return [...items.values()].sort(
    (a, b) => b.date.localeCompare(a.date) || b.id.localeCompare(a.id),
  );
}

export const stageLabels: Record<string, string> = {
  batch_umm: "获取目录",
  json_index: "核对索引",
  batch_parse: "校验元数据",
  concept_recovery: "补取元数据",
};
