// Creation has already succeeded. A display refresh must never gate starting IDs.
export async function finishSubmission(
  ids: string[], start: (ids: string[]) => Promise<void>,
  refresh: () => Promise<void>, onSyncError: (error: unknown) => void,
) {
  try { if (ids.length) await start(ids); }
  finally { try { await refresh(); } catch (error) { onSyncError(error); } }
}
