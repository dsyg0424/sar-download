import { useState, type ReactNode } from "react";
import { RegionActionsContext } from "../../map/RegionActionsContext";
import { ArrowRight, Loader2, Search } from "lucide-react";
import { DatePicker } from "../../../shared/components/DatePicker";
import { Select } from "../../../shared/components/Select";
import { useSarSearchStore } from "../store/useSarSearchStore";
import {
  imageTypeOptions,
  nisarOnly,
  optionsFor,
  satelliteOptions,
  directionOptions,
} from "../model/searchOptions";

export function SarSearchForm({
  areaSelector,
  onReset,
  adding,
}: {
  areaSelector: ReactNode;
  onReset: () => void;
  adding: boolean;
}) {
  const store = useSarSearchStore();
  const [actionsHost, setActionsHost] = useState<HTMLDivElement | null>(null);
  const { polarizations: polarizationOptions } = optionsFor(store.request);
  return (
    <form
      noValidate
      role="search"
      aria-label="SAR 影像检索"
      className="sar-search-form"
      onSubmit={(event) => {
        event.preventDefault();
        void store.search();
      }}
    >
      <RegionActionsContext.Provider value={actionsHost}>
      <fieldset disabled={store.busy} aria-label="检索条件">
        {areaSelector}
        <div className="search-criteria">
        <div ref={setActionsHost} className="region-actions-host" role="group" aria-label="范围选择方式" />
        <div className="date-range-field" role="group" aria-label="采集日期范围">
          <span className="infield-label">采集日期</span>
          <div className="date-range-inputs">
            <DatePicker
              label="开始日期"
              displayFormat="dots"
              value={store.request.start}
              onChange={(value) => store.update({ start: value })}
              max={store.request.end || undefined}
              rangeStart={store.request.start}
              rangeEnd={store.request.end}
              disabled={store.busy}
            />
            <ArrowRight size={12} aria-hidden="true" />
            <DatePicker
              label="结束日期"
              displayFormat="dots"
              value={store.request.end}
              onChange={(value) => store.update({ end: value })}
              min={store.request.start || undefined}
              rangeStart={store.request.start}
              rangeEnd={store.request.end}
              disabled={store.busy}
            />
          </div>
        </div>
        <div className="sar-parameter-grid">
          <label className="parameter-field">
            <span className="infield-label">产品类型</span>
            <Select
              aria-label="产品类型"
              multiple
              emptyOptionLabel="不限"
              showSelectionMark={false}
              disabled={store.busy}
              value={store.request.productTypes ?? []}
              onChange={(e) =>
                store.update({
                  productTypes: Array.from(
                    e.target.selectedOptions,
                    (o) => o.value,
                  ).filter(Boolean),
                })
              }
            >
              {imageTypeOptions
                .filter((o) => o.value)
                .map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
            </Select>
          </label>
          <label className="parameter-field">
            <span className="infield-label">S1 卫星</span>
            <Select
              aria-label="S1 卫星"
              multiple
              emptyOptionLabel="不限"
              showSelectionMark={false}
              disabled={store.busy || nisarOnly(store.request)}
              value={store.request.satellites ?? []}
              onChange={(e) =>
                store.update({
                  satellites: Array.from(
                    e.target.selectedOptions,
                    (o) => o.value,
                  ).filter(Boolean),
                })
              }
            >
              {satelliteOptions
                .filter((o) => o.value.startsWith("Sentinel-1"))
                .map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
            </Select>
          </label>
          <label className="parameter-field">
            <span className="infield-label">轨道方向</span>
            <Select
              aria-label="轨道方向"
              title={
                directionOptions.find(
                  (o) => o.value === store.request.direction,
                )?.label
              }
              value={store.request.direction}
              onChange={(e) => store.update({ direction: e.target.value })}
            >
              {directionOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </Select>
          </label>
          <label className="parameter-field">
            <span className="infield-label">极化方式</span>
            <Select
              aria-label="极化方式"
              title={
                polarizationOptions.find(
                  (o) => o.value === store.request.polarization,
                )?.label
              }
              disabled={nisarOnly(store.request)}
              value={store.request.polarization}
              onChange={(e) => store.update({ polarization: e.target.value })}
            >
              {polarizationOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </Select>
          </label>
        </div>
        </div>
      </fieldset>
      <div className="sar-search-actions">
        {store.busy && <button type="button" className="btn btn-secondary" disabled={store.cancelling} onClick={() => void store.cancel()}>{store.cancelling ? "取消中…" : "取消检索"}</button>}
        <button
          type="button"
          className="btn btn-secondary"
          disabled={store.busy || adding}
          onClick={() => {
            onReset();
          }}
          title="清空筛选条件、结果和勾选，不删除下载任务"
        >
          重置
        </button>
        <button className="btn btn-primary" disabled={store.busy}>
          {store.busy ? (
            <Loader2 size={14} className="spinner" />
          ) : (
            <Search size={14} />
          )}
          {store.busy ? "检索中…" : "检索"}
        </button>
      </div>
      </RegionActionsContext.Provider>
    </form>
  );
}
