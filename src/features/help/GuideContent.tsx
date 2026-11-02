import { Fragment, type ReactNode } from "react";

// Render the guide's headings, lists, tables, bold and code as React text nodes.
// Raw HTML is never evaluated and the Markdown document remains the only source.
export function inline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).map((part, i) =>
    part.startsWith("**") && part.endsWith("**") ? <strong key={i}>{part.slice(2, -2)}</strong> :
    part.startsWith("`") && part.endsWith("`") ? <code key={i}>{part.slice(1, -1)}</code> : <Fragment key={i}>{part}</Fragment>);
}
const cells = (line: string) => line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(s => s.trim());
export function guideSections(source: string) {
  return source.split(/\r?\n/).flatMap((line, index) => line.startsWith("## ") ? [{ id: `guide-${index}`, title: line.slice(3) }] : []);
}
export function GuideContent({ source }: { source: string }) {
  const lines = source.split(/\r?\n/), output: ReactNode[] = [];
  for (let i = 0; i < lines.length;) {
    const line = lines[i].trim(), key = i;
    if (!line) { i++; continue; }
    const heading = /^(#{1,6}) (.+)$/.exec(line);
    if (heading) {
      const Tag = `h${Math.min(heading[1].length + 1, 6)}` as "h2" | "h3" | "h4" | "h5" | "h6";
      output.push(<Tag key={key} id={`guide-${i}`}>{inline(heading[2])}</Tag>); i++; continue;
    }
    if (line.startsWith("|") && /^\s*\|?\s*:?-{3,}/.test(lines[i+1] ?? "")) {
      const headings = cells(line); i += 2; const rows: string[][] = [];
      while (i < lines.length && lines[i].trim().startsWith("|")) rows.push(cells(lines[i++]));
      output.push(<div key={key} className="help-table"><table><thead><tr>{headings.map((cell,n)=><th scope="col" key={n}>{inline(cell)}</th>)}</tr></thead>
        <tbody>{rows.map((row,n)=><tr key={n}>{row.map((cell,c)=><td key={c}>{inline(cell)}</td>)}</tr>)}</tbody></table></div>); continue;
    }
    const ordered = /^\d+\. /.test(line);
    if (ordered || line.startsWith("- ")) {
      const items: ReactNode[] = [], start = ordered ? Number(line.split(".")[0]) : undefined;
      const match = ordered ? /^\d+\. / : /^- /;
      while (i < lines.length && match.test(lines[i].trim())) { items.push(<li key={i}>{inline(lines[i].trim().replace(match,""))}</li>); i++; }
      output.push(ordered ? <ol key={key} start={start}>{items}</ol> : <ul key={key}>{items}</ul>); continue;
    }
    const paragraph = [line]; i++;
    while (i < lines.length && lines[i].trim() && !/^(#|\||- |\d+\. )/.test(lines[i].trim())) paragraph.push(lines[i++].trim());
    output.push(<p key={key}>{inline(paragraph.join(" "))}</p>);
  }
  return <>{output}</>;
}
