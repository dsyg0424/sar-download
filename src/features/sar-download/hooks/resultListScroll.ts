/** Scroll only the result viewport, accounting for its sticky group heading. */
export function revealResultRow(viewport: HTMLElement, target: HTMLElement, headingHeight: number) {
  const view = viewport.getBoundingClientRect();
  const row = target.getBoundingClientRect();
  const top = view.top + viewport.clientTop + headingHeight + 4;
  const bottom = view.top + viewport.clientTop + viewport.clientHeight - 4;
  if (row.top < top) viewport.scrollTop += row.top - top;
  else if (row.bottom > bottom) viewport.scrollTop += row.bottom - bottom;
}
