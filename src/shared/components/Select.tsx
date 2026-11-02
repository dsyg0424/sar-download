import { useEffect, useId, useRef, useState, type SelectHTMLAttributes } from "react";
import { createPortal } from "react-dom";
import { ChevronDown } from "lucide-react";
import "./select.css";
type Choice = { value: string; label: string; disabled: boolean; hidden: boolean };
export function Select({ children, className = "", style, showSelectionMark = true, emptyOptionLabel, ...props }: SelectHTMLAttributes<HTMLSelectElement> & { showSelectionMark?: boolean; emptyOptionLabel?: string }) {
  const native = useRef<HTMLSelectElement>(null), trigger = useRef<HTMLButtonElement>(null), menu = useRef<HTMLDivElement>(null);
  const id = useId();
  const [open, setOpen] = useState(false), [label, setLabel] = useState("");
  const [name, setName] = useState<string>();
  const [choices, setChoices] = useState<Choice[]>([]), [active, setActive] = useState(0);
  const [position, setPosition] = useState({ left: 0, top: 0, width: 0, maxHeight: 180 });
  const dialog = trigger.current?.closest("dialog");
  useEffect(() => {
    if (open && dialog && menu.current?.showPopover) menu.current.showPopover();
  }, [open, dialog]);
  const sync = () => {
    const selected = Array.from(native.current?.selectedOptions ?? []).map(o => o.text);
    setLabel(props.multiple ? selected.length > 1 ? `已选 ${selected.length} 项` : selected[0] ?? "不限" : selected[0] ?? "请选择");
  };
  useEffect(() => {
    sync();
    const labels = native.current?.labels;
    setName(props["aria-label"] ?? (labels?.length ? Array.from(labels).map(l => {
      const copy = l.cloneNode(true) as HTMLElement;
      copy.querySelectorAll(".ui-select").forEach(n => n.remove());
      return copy.textContent?.trim();
    }).join(" ") : native.current?.parentElement?.previousElementSibling?.tagName === "LABEL" ? native.current.parentElement.previousElementSibling.textContent?.trim() : props.title));
    if (native.current?.matches(":disabled")) setOpen(false);
  }, [props.value, props.disabled, children, props["aria-label"]]);
  const choose = (index: number) => {
    if (!native.current || native.current.matches(":disabled") || choices[index]?.disabled || !choices[index]) return;
    if (props.multiple) {
      const options = Array.from(native.current.options);
      if (emptyOptionLabel && choices[index].value === "") {
        options.forEach(o => { o.selected = o.value === ""; });
      } else {
        options[index].selected = !options[index].selected;
        if (emptyOptionLabel) {
          const hasSelection = options.some(o => o.value !== "" && o.selected);
          options.forEach(o => { if (o.value === "") o.selected = !hasSelection; });
        }
      }
      setActive(index);
    }
    else native.current.value = choices[index].value;
    native.current.dispatchEvent(new Event("change", { bubbles: true }));
    sync(); if (!props.multiple) setOpen(false); trigger.current?.focus();
  };
  const show = () => {
    if (!native.current || native.current.matches(":disabled")) return;
    const items = Array.from(native.current.options).map(o => ({ value: o.value, label: o.text, hidden: Boolean(o.hidden), disabled: o.disabled || Boolean(o.hidden) || (o.parentElement instanceof HTMLOptGroupElement && o.parentElement.disabled) }));
    setChoices(items); setActive(items[native.current.selectedIndex]?.hidden ? Math.max(0, items.findIndex(item => !item.disabled)) : Math.max(0, native.current.selectedIndex));
    const r = trigger.current!.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 10;
    const maxHeight = Math.max(28, Math.min(180, Math.max(below, r.top - 10)));
    const height = Math.min(maxHeight, items.filter(item => !item.hidden).length * 30 + 8), width = Math.min(Math.max(r.width, 120), window.innerWidth - 10);
    setPosition({ left: Math.max(5, Math.min(r.left, window.innerWidth - width - 5)), top: below >= height ? r.bottom + 5 : Math.max(5, r.top - height - 5), width, maxHeight }); setOpen(true);
  };
  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => { if (!menu.current?.contains(e.target as Node) && !trigger.current?.contains(e.target as Node)) setOpen(false); };
    const scroll = (e: Event) => { if (!menu.current?.contains(e.target as Node)) setOpen(false); };
    const close = () => setOpen(false);
    document.addEventListener("pointerdown", outside); document.addEventListener("scroll", scroll, true);
    window.addEventListener("resize", close); window.addEventListener("blur", close);
    return () => { document.removeEventListener("pointerdown", outside); document.removeEventListener("scroll", scroll, true); window.removeEventListener("resize", close); window.removeEventListener("blur", close); };
  }, [open]);
  useEffect(() => { if (open) document.getElementById(`${id}-${active}`)?.scrollIntoView({ block: "nearest" }); }, [active, open, id]);
  useEffect(() => {
    if (!open || !native.current) return;
    const items = Array.from(native.current.options).map(option => ({ value: option.value, label: option.text, hidden: Boolean(option.hidden),
      disabled: option.disabled || Boolean(option.hidden) || (option.parentElement?.tagName === "OPTGROUP" && (option.parentElement as HTMLOptGroupElement).disabled) }));
    setChoices(items);
    setActive(index => Math.min(index, Math.max(0, items.length - 1)));
  }, [children, open]);
  return <span className="ui-select" style={style}>
    <select {...props} value={props.multiple && emptyOptionLabel && Array.isArray(props.value) && !props.value.length ? [""] : props.value} ref={native} hidden style={{ display: "none" }} tabIndex={-1} aria-hidden="true" onChange={e => { props.onChange?.(e); sync(); }} onInvalid={e => { e.preventDefault(); trigger.current?.focus(); props.onInvalid?.(e); }}>{props.multiple && emptyOptionLabel && <option value="">{emptyOptionLabel}</option>}{children}</select>
    <button ref={trigger} type="button" className={`ui-select-trigger ${className} ${open ? "open" : ""}`} disabled={props.disabled}
      title={props.title ?? (props.multiple ? Array.from(native.current?.selectedOptions ?? []).map(o => o.text).join("、") || "不限" : label)} role="combobox" aria-label={name} aria-labelledby={props["aria-labelledby"]} aria-expanded={open} aria-controls={open ? id : undefined} aria-haspopup="listbox" aria-activedescendant={open ? `${id}-${active}` : undefined}
      onClick={() => open ? setOpen(false) : show()} onBlur={() => setOpen(false)} onKeyDown={e => {
        if (e.key === "Escape") { if (open) e.stopPropagation(); setOpen(false); return; }
        if (e.key === "Tab") { setOpen(false); return; }
        if (["ArrowDown", "ArrowUp", "Home", "End", "Enter", " "].includes(e.key)) {
          e.preventDefault(); if (!open) { show(); return; }
          if (e.key === "Enter" || e.key === " ") { choose(active); return; }
          const enabled = choices.map((o, i) => o.disabled ? -1 : i).filter(i => i >= 0);
          if (!enabled.length) return;
          const index = enabled.indexOf(active);
          setActive(e.key === "Home" ? enabled[0] : e.key === "End" ? enabled[enabled.length - 1] : enabled[(index + (e.key === "ArrowUp" ? -1 : 1) + enabled.length) % enabled.length]);
        }
      }}><span>{label}</span><ChevronDown size={12} /></button>
    {open && createPortal(<div ref={menu} id={id} popover={dialog ? "manual" : undefined} role="listbox" aria-multiselectable={props.multiple || undefined} aria-label={name} className="ui-select-menu" style={position} onPointerDown={e => e.preventDefault()}>
      {choices.map((o, i) => o.hidden ? null : <div id={`${id}-${i}`} key={i} role="option" aria-selected={native.current?.options[i]?.selected ?? false} aria-disabled={o.disabled}
        className={`ui-select-option ${native.current?.options[i]?.selected ? "selected" : ""} ${i === active ? "active" : ""}`} title={o.label} onClick={() => choose(i)}>{props.multiple && showSelectionMark && <span aria-hidden="true">{native.current?.options[i]?.selected ? "☑" : "☐"} </span>}{o.label}</div>)}
    </div>, dialog ?? document.body)}
  </span>;
}
