import { useEffect, useMemo, useRef } from "react";
import { X } from "lucide-react";
import { GuideContent, guideSections } from "./GuideContent";
import { markHelpShown } from "./useHelp";
import "./help.css";

export function HelpDialog({ source, onClose }: { source: string; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const sections = useMemo(() => guideSections(source), [source]);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const element = dialog.current!;
    if (!element.open) element.showModal();
    markHelpShown(); close.current?.focus();
    return () => { if (element.open) element.close(); previous?.focus({preventScroll:true}); };
  }, []);
  return <dialog ref={dialog} className="help-dialog" aria-labelledby="help-heading"
    onCancel={event => { event.preventDefault(); onClose(); }} onKeyDown={event => event.stopPropagation()}>
    <header className="dialog-heading"><h2 id="help-heading">使用帮助</h2>
      <button ref={close} type="button" className="icon-button" aria-label="关闭帮助" title="关闭帮助" onClick={onClose}><X size={18}/></button></header>
    <div className="help-layout">
      <nav className="help-toc" aria-label="帮助目录">{sections.map(section => <button type="button" key={section.id}
        onClick={() => dialog.current?.querySelector(`#${section.id}`)?.scrollIntoView({block:"start"})}>{section.title}</button>)}</nav>
      <article className="help-content" tabIndex={0} aria-label="新用户操作指南"><GuideContent source={source}/></article>
    </div>
    <footer><span>随时可通过标题栏“帮助”再次查看。</span><button type="button" className="btn btn-primary" onClick={onClose}>开始使用</button></footer>
  </dialog>;
}
