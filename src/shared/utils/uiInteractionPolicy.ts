export function installUiInteractionPolicy(document: Document): () => void {
  const contextMenu = (event: Event) => {
    event.preventDefault();
    event.stopPropagation();
  };
  const select = (event: Event) => {
    const node = event.target as Node | null;
    const element = node?.nodeType === 1 ? node as Element : node?.parentElement;
    if (!element?.closest('input, textarea, [contenteditable="true"], [contenteditable="plaintext-only"]')) {
      event.preventDefault();
    }
  };
  // Capture at document level so dialogs, portals and map surfaces follow the same policy.
  document.addEventListener("contextmenu", contextMenu, true);
  document.addEventListener("selectstart", select, true);
  return () => {
    document.removeEventListener("contextmenu", contextMenu, true);
    document.removeEventListener("selectstart", select, true);
  };
}
