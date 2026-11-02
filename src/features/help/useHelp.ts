import { useCallback, useState } from "react";

export const HELP_SEEN_KEY = "sar-download.help.seen";
let shownThisSession = false;
export function shouldShowHelp() {
  if (shownThisSession) return false;
  try { return localStorage.getItem(HELP_SEEN_KEY) !== "1"; }
  catch { return true; }
}
export function markHelpShown() {
  shownThisSession = true;
  try { localStorage.setItem(HELP_SEEN_KEY, "1"); } catch { /* Still usable if storage is unavailable. */ }
}
export function useHelp() {
  const [open, setOpen] = useState(shouldShowHelp);
  return { open, show: useCallback(() => setOpen(true), []), close: useCallback(() => setOpen(false), []) };
}
