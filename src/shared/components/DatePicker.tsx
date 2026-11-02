import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";
import "./date-picker.css";
import { localDate } from "../utils/localDate";

export function isCalendarDate(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || value.startsWith("0000")) return false;
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}
export function isCalendarMonth(value: string) {
  return /^\d{4}-\d{2}$/.test(value) && isCalendarDate(`${value}-01`);
}
export function calendarDays(month: string): string[] {
  const first = new Date(`${month}-01T00:00:00Z`);
  const offset = (first.getUTCDay() + 6) % 7;
  return Array.from({ length: 42 }, (_, i) => {
    const date = new Date(first); date.setUTCDate(1 - offset + i);
    return date.toISOString().slice(0, 10);
  });
}
export function DatePicker({ value, onChange, label, min, max, rangeStart, rangeEnd, disabled = false, title, mode = "date", calendarMin = "2014-01-01", calendarMax, displayFormat = "iso" }: {
  value: string; onChange: (value: string) => void; label: string;
  min?: string; max?: string; rangeStart?: string; rangeEnd?: string; disabled?: boolean; title?: string;
  mode?: "date" | "month"; calendarMin?: string; calendarMax?: string; displayFormat?: "iso" | "dots";
}) {
  const [today, setToday] = useState(localDate);
  useEffect(() => {
    const refresh = () => setToday(localDate());
    const timer = setInterval(refresh, 1000);
    window.addEventListener("focus", refresh);
    return () => { clearInterval(timer); window.removeEventListener("focus", refresh); };
  }, []);
  const firstDate = isCalendarDate(calendarMin) ? calendarMin : "2014-01-01";
  const lastDate = calendarMax && isCalendarDate(calendarMax) ? calendarMax : today;
  const firstYear = Number(firstDate.slice(0, 4)), lastYear = Number(lastDate.slice(0, 4));
  const valid = mode === "month" ? isCalendarMonth : isCalendarDate;
  const lower = min && valid(min) ? (mode === "month" ? `${min}-01` : min) : firstDate;
  const upper = max && valid(max) ? (mode === "month" ? `${max}-31` : max) : lastDate;
  const lowerDate = lower > firstDate ? lower : firstDate, upperDate = upper < lastDate ? upper : lastDate;
  const input = useRef<HTMLInputElement>(null), anchor = useRef<HTMLSpanElement>(null), popup = useRef<HTMLDivElement>(null);
  const original = useRef(value), id = useId();
  const [open, setOpen] = useState(false), [error, setError] = useState("");
  const [month, setMonth] = useState(today.slice(0, 7));
  const [view, setView] = useState<"days" | "years" | "months">("days");
  const [yearPage, setYearPage] = useState(Number(today.slice(0, 4)) - 5);
  const yearList = useRef<HTMLDivElement>(null);
  const [yearRow, setYearRow] = useState(0);
  useEffect(() => {
    if (open && view === "years" && yearList.current) {
      const row = Math.max(0, Math.min(Math.ceil((lastYear - firstYear + 1) / 3) - 4, Math.floor((yearPage - firstYear) / 3)));
      yearList.current.scrollTop = row * 56;
      setYearRow(row);
    }
  }, [open, view, yearPage, firstYear, lastYear]);
  const [position, setPosition] = useState({ left: 0, top: 0, maxHeight: 340 });
  const allowed = (day: string) => valid(day) && (mode === "month"
    ? day >= lowerDate.slice(0, 7) && day <= upperDate.slice(0, 7) && lowerDate <= upperDate
    : day >= lowerDate && day <= upperDate);
  const validate = () => {
    const next = value.trim();
    if (!next) { setError(""); return true; }
    if (!valid(next)) { setError(mode === "month" ? "请输入有效月份 YYYY-MM" : "请输入有效日期 YYYY-MM-DD"); return false; }
    if (!allowed(next)) { setError("日期超出起止范围"); return false; }
    setError(""); if (next !== value) onChange(next); return true;
  };
  useEffect(() => { if (!value || allowed(value)) setError(""); }, [value, min, max, firstDate, lastDate, mode]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!anchor.current?.contains(event.target as Node) && !popup.current?.contains(event.target as Node)) setOpen(false); };
    const scroll = (event: Event) => { if (!popup.current?.contains(event.target as Node)) setOpen(false); };
    const close = () => setOpen(false);
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false); input.current?.focus(); } };
    document.addEventListener("pointerdown", outside); document.addEventListener("scroll", scroll, true); document.addEventListener("keydown", escape, true);
    window.addEventListener("resize", close); window.addEventListener("blur", close);
    const frame = requestAnimationFrame(() => popup.current?.querySelector<HTMLButtonElement>('button[aria-pressed="true"], .date-day:not(:disabled)')?.focus());
    return () => { cancelAnimationFrame(frame); document.removeEventListener("pointerdown", outside); document.removeEventListener("scroll", scroll, true); document.removeEventListener("keydown", escape, true); window.removeEventListener("resize", close); window.removeEventListener("blur", close); };
  }, [open]);
  const show = () => {
    if (input.current?.matches(":disabled")) return;
    const candidate = valid(value) ? (mode === "month" ? `${value}-01` : value) : today;
    const initial = candidate < lowerDate ? lowerDate : candidate > upperDate ? upperDate : candidate;
    setMonth(initial.slice(0, 7));
    setView(mode === "month" ? "months" : "days");
    setYearPage(Math.max(firstYear, Math.min(lastYear, Number(initial.slice(0, 4)) - 5)));
    const rect = anchor.current!.getBoundingClientRect(), below = window.innerHeight - rect.bottom - 10;
    const height = Math.min(340, Math.max(below, rect.top - 10));
    setPosition({ left: Math.max(5, Math.min(rect.left, window.innerWidth - Math.min(280, window.innerWidth - 10) - 5)), top: below >= height ? rect.bottom + 5 : Math.max(5, rect.top - height - 5), maxHeight: height });
    setOpen(true);
  };
  const moveMonth = (delta: number) => {
    if (view === "years") { setYearPage(Math.max(firstYear, Math.min(lastYear, firstYear + Math.floor((yearList.current?.scrollTop ?? 0) / 56) * 3 + delta * 12))); return; }
    if (view === "months") { setMonth(m => `${String(Math.max(firstYear, Math.min(lastYear, Number(m.slice(0, 4)) + delta))).padStart(4, "0")}-${m.slice(5)}`); return; }
    const date = new Date(`${month}-01T00:00:00Z`); date.setUTCMonth(date.getUTCMonth() + delta);
    const next = date.toISOString().slice(0, 7);
    if (next >= firstDate.slice(0, 7) && next <= lastDate.slice(0, 7)) setMonth(next);
  };
  const choose = (day: string) => { if (input.current?.matches(":disabled") || (day && !allowed(day))) return; onChange(day); setError(""); setOpen(false); input.current?.focus(); original.current = day; };
  return <span className="date-picker" ref={anchor}>
    <span className="date-input-wrap">
      <input className="date-text-input" ref={input} aria-label={label} aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? id : undefined} aria-invalid={!!error} aria-describedby={error ? `${id}-error` : undefined} value={displayFormat === "dots" && mode === "date" ? value.replace(/-/g, ".") : value} disabled={disabled} placeholder={mode === "month" ? "选择月份" : "选择日期"} title={title ?? (mode === "month" ? "YYYY-MM" : displayFormat === "dots" ? "YYYY.MM.DD" : "YYYY-MM-DD")} maxLength={mode === "month" ? 7 : 10} onClick={show}
        onFocus={() => { original.current = value; }} onChange={e => { onChange(displayFormat === "dots" && mode === "date" ? e.target.value.replace(/\./g, "-") : e.target.value); setError(""); }} onBlur={validate}
        onKeyDown={e => { if (e.key === "Enter") { e.preventDefault(); if (validate()) setOpen(false); } else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onChange(original.current); setError(""); setOpen(false); } else if (e.altKey && e.key === "ArrowDown") { e.preventDefault(); show(); } }} />
      <button type="button" className="date-open" aria-label={`选择${label}`} aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? id : undefined} disabled={disabled} onClick={() => open ? setOpen(false) : show()}><CalendarDays size={14} /></button>
    </span>
    {error && <span id={`${id}-error`} className="date-error" role="alert">{error}</span>}
    {open && createPortal(<div ref={popup} id={id} className="date-calendar" role="dialog" aria-label={`${label}日历`} style={position}
      onBlur={e => { if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node) && !anchor.current?.contains(e.relatedTarget as Node)) setOpen(false); }}>
      <div className="date-calendar-header">
        <button type="button" aria-label={view === "years" ? "上一组年份" : view === "months" ? "上一年" : "上个月"} onClick={() => moveMonth(-1)}><ChevronLeft size={16} /></button>
        <span className="date-calendar-titles"><button type="button" aria-label="选择年份" aria-expanded={view === "years"} onClick={() => { setYearPage(Math.max(firstYear, Math.min(lastYear, Number(month.slice(0, 4)) - 5))); setView(v => v === "years" ? mode === "month" ? "months" : "days" : "years"); }}>{Number(month.slice(0, 4))}年</button><button type="button" aria-label="选择月份" aria-expanded={view === "months"} onClick={() => setView(v => mode === "month" ? "months" : v === "months" ? "days" : "months")}>{Number(month.slice(5))}月</button></span>
        <button type="button" aria-label={view === "years" ? "下一组年份" : view === "months" ? "下一年" : "下个月"} onClick={() => moveMonth(1)}><ChevronRight size={16} /></button>
      </div>
      <div className="date-calendar-body">
      {view === "years" && <div className="date-year-scroll" ref={yearList} aria-label="年份列表" onScroll={e => setYearRow(Math.floor(e.currentTarget.scrollTop / 56))}>
        <div className="date-year-space" style={{ height: Math.ceil((lastYear + 1 - firstYear) / 3) * 56 }}>
          {Array.from({ length: 24 }, (_, i) => firstYear + Math.max(0, yearRow - 2) * 3 + i).filter(year => year <= lastYear).map(year => <button type="button" key={year} style={{ top: Math.floor((year - firstYear) / 3) * 56, left: `${((year - firstYear) % 3) * 100 / 3}%` }} aria-pressed={year === Number(month.slice(0, 4))} onClick={() => { setMonth(`${String(year).padStart(4, "0")}-${month.slice(5)}`); setView("months"); }}>{year}年</button>)}
        </div>
      </div>}
      {view === "months" && <div className="date-period-grid" aria-label="月份列表">{Array.from({ length: 12 }, (_, i) => i + 1).map(m => {
        const candidate = `${month.slice(0, 4)}-${String(m).padStart(2, "0")}`;
        const enabled = candidate >= lowerDate.slice(0, 7) && candidate <= upperDate.slice(0, 7) && lowerDate <= upperDate;
        return <button type="button" key={m} disabled={!enabled || disabled} aria-pressed={m === Number(month.slice(5))} onClick={() => { if (mode === "month") choose(candidate); else { setMonth(candidate); setView("days"); } }}>{m}月</button>;
      })}</div>}
      {view === "days" && <>
      <div className="date-weekdays">{["一", "二", "三", "四", "五", "六", "日"].map(day => <span key={day}>{day}</span>)}</div>
      <div className="date-grid">{calendarDays(month).map(day => <button type="button" key={day} className={`date-day ${day.slice(0, 7) !== month ? "outside" : ""} ${day === today ? "today" : ""} ${day === value ? "selected" : ""} ${rangeStart && rangeEnd && day >= rangeStart && day <= rangeEnd ? "in-range" : ""}`} aria-label={day} aria-pressed={day === value} aria-current={day === today ? "date" : undefined} disabled={!allowed(day) || disabled} onClick={() => choose(day)}>{Number(day.slice(8))}</button>)}</div>
      </>}
      </div>
      <div className="date-calendar-footer"><button type="button" onClick={() => choose("")}>清空</button><span>{calendarMax ? "" : "截止本地今天"}</span><button type="button" disabled={!allowed(mode === "month" ? today.slice(0, 7) : today)} onClick={() => choose(mode === "month" ? today.slice(0, 7) : today)}>{mode === "month" ? "本月" : "今天"}</button></div>
    </div>, document.body)}
  </span>;
}
